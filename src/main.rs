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

use rusttracker::{audio, engine, ipc, lyrics, playlist, state, touch, ui};

use crate::engine::{EngineAction, VulkanEngine};
use crate::state::AppState;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(
        value_name = "FILE_OR_URL",
        help = "Media file(s), tracker module(s), playlist(s), folder(s), or stream URL(s) to open"
    )]
    file: Vec<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Launch in terminal user interface (TUI) mode instead of the Vulkan/wgpu GUI"
    )]
    tui: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Capture live audio from the system microphone / loopback input device"
    )]
    mic: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "windowed",
        help = "Launch in borderless fullscreen mode"
    )]
    fullscreen: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "fullscreen",
        help = "Force windowed mode (overrides automatic fullscreen under Gamescope)"
    )]
    windowed: bool,

    #[arg(
        long,
        value_name = "ID_OR_NAME",
        num_args = 0..=1,
        default_missing_value = "",
        help = "Start directly on the specified visualizer ID or short name (pass empty or 'list' to list visualizers)"
    )]
    vis: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "List all available visualizers with their IDs, short names, and descriptions, then exit"
    )]
    list_vis: bool,

    #[arg(
        long,
        value_name = "FRAMES",
        num_args = 0..=1,
        default_missing_value = "300",
        help = "Run automated benchmark for FRAMES frames (default: 300) and print telemetry"
    )]
    bench: Option<u32>,

    #[arg(
        long,
        default_value_t = false,
        help = "Disable VSync / display sync and unlock uncapped frame rates"
    )]
    uncapped: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["passthrough", "surround_upmix", "mpegh"],
        help = "Force 2.0 stereo downmix output even on multi-channel hardware endpoints"
    )]
    stereo: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["stereo", "surround_upmix", "mpegh"],
        help = "Enable hardware bitstream passthrough (WASAPI Exclusive / PipeWire) for compressed surround formats"
    )]
    passthrough: bool,

    #[arg(
        long,
        visible_alias = "upmix",
        default_value_t = false,
        conflicts_with_all = ["no_surround_upmix", "stereo", "passthrough"],
        help = "Enable automatic 7.1.4 surround & height upmixing with configurable crossover EQ"
    )]
    surround_upmix: bool,

    #[arg(
        long,
        visible_alias = "no-upmix",
        default_value_t = false,
        conflicts_with = "surround_upmix",
        help = "Disable automatic surround & height upmixing on multi-channel hardware endpoints"
    )]
    no_surround_upmix: bool,

    #[arg(
        long,
        visible_alias = "mpegh-714",
        default_value_t = false,
        conflicts_with_all = ["no_mpegh", "stereo", "passthrough"],
        help = "Encode 7.1.4 (11.1, CICP Layout 19) output as an MPEG-H 3D Audio IEC 61937-13 HDMI bitstream"
    )]
    mpegh: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "mpegh",
        help = "Disable MPEG-H 3D Audio 7.1.4 (11.1) HDMI bitstreaming and use uncompressed LPCM only"
    )]
    no_mpegh: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Headlessly register RustTracker as a Non-Steam shortcut with SteamOS Home Screen artwork and exit (ideal for SSH deployment)"
    )]
    register_steam: bool,
}

pub(crate) fn env_flag_enabled(val: Option<&str>) -> bool {
    val.is_some_and(|v| {
        let trimmed = v.trim();
        !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
    })
}

pub(crate) fn should_prefer_x11_backend(
    wayland_env: Option<&str>,
    x11_env: Option<&str>,
    has_wayland_display: bool,
    has_x11_display: bool,
) -> bool {
    let force_x11 = env_flag_enabled(x11_env);
    let disable_wayland = wayland_env.is_some_and(|v| {
        let trimmed = v.trim();
        trimmed == "0" || trimmed.eq_ignore_ascii_case("false")
    });
    let use_wayland = !force_x11 && !disable_wayland && has_wayland_display;
    !use_wayland && (has_x11_display || force_x11)
}

pub(crate) fn has_cli_upmix_override(args: &Args) -> bool {
    args.surround_upmix
        || args.no_surround_upmix
        || args.stereo
        || (args.mpegh && !args.no_surround_upmix)
}

fn apply_cli_audio_flags(state: &mut AppState, args: &Args) {
    if args.stereo {
        state.set_force_stereo_downmix(true);
    }
    if args.passthrough {
        if cfg!(any(target_os = "windows", target_os = "linux")) {
            state.passthrough_enabled = true;
            state.surround_upmix.enabled = false;
            state.force_stereo_downmix = false;
        } else {
            state.passthrough_enabled = false;
            state.osd_text = Some("Bitstream Passthrough requires Linux or Windows".to_string());
            state.osd_timer = 2.5;
        }
    }
    if !args.stereo {
        if args.no_surround_upmix {
            state.set_surround_upmix(false);
        } else if args.surround_upmix {
            state.set_surround_upmix(true);
        }
    }
    if args.no_mpegh {
        state.set_mpegh_bitstream(false);
    } else if args.mpegh {
        if !args.no_surround_upmix && !args.stereo {
            state.set_surround_upmix(true);
        }
        state.set_mpegh_bitstream(true);
    }
    if args.stereo {
        // Ensure combinable flags like `--stereo --no-mpegh` or `--stereo --no-surround-upmix`
        // never overwrite the forced-stereo OSD with LPCM/upmix messages.
        state.osd_text = Some("🎧 Stereo Downmix (2.0): Forced".to_string());
        state.osd_timer = 2.5;
    }
}

/// Updates startup OSD after `audio::start_audio_thread` probes real hardware channels
/// (`apply_cli_audio_flags` runs before stream open while `hardware_channels` is still 2).
pub(crate) fn reconcile_post_load_cli_audio_osd(state: &mut AppState, args: &Args) {
    if args.stereo {
        state.osd_text = Some("🎧 Stereo Downmix (2.0): Forced".to_string());
        state.osd_timer = 2.5;
    } else if args.surround_upmix && !args.mpegh && state.hardware_channels > 2 {
        if state.surround_upmix.enabled {
            let badge = crate::state::SurroundUpmixConfig::layout_badge(state.hardware_channels);
            state.osd_text = Some(format!("🔊 Surround Upmix: ON ({badge})"));
            state.osd_timer = 2.5;
        } else if state.is_native_atmos {
            state.osd_text = Some("🔊 Surround Upmix: Bypassed (Native Dolby Atmos)".to_string());
            state.osd_timer = 2.5;
        } else if !crate::state::SurroundUpmixConfig::can_upmix(
            state.source_channels,
            state.hardware_channels,
        ) {
            state.osd_text = Some(format!(
                "🔊 Surround Upmix: Bypassed (Native {}ch Match)",
                state.source_channels
            ));
            state.osd_timer = 2.5;
        }
    }
}

pub(crate) fn reconcile_passthrough_change_osd(state: &mut AppState, pt_enabled: bool) {
    let existing = state.osd_text.as_deref().unwrap_or_default();
    if state.force_stereo_downmix || existing.contains("Stereo Downmix") {
        if state.force_stereo_downmix {
            state.osd_text = Some("🎧 Stereo Downmix (2.0): Forced".to_string());
        }
        return;
    }
    if state.surround_upmix.enabled && !pt_enabled {
        if !existing.contains("MPEG-H") && !existing.contains("11.1") {
            if state.hardware_channels > 2 {
                let badge =
                    crate::state::SurroundUpmixConfig::layout_badge(state.hardware_channels);
                state.osd_text = Some(format!("🔊 Surround Upmix: ON ({badge})"));
            } else {
                state.osd_text =
                    Some("🔊 Surround Upmix: Armed (Stereo/Mono Output — No Change)".to_string());
            }
        }
    } else if !pt_enabled && (existing.contains("Surround Upmix") || existing.contains("11.1")) {
        // Preserve explicit Surround Upmix OFF or 11.1 MPEG-H OFF OSD across shared-mode reload
    } else {
        state.osd_text = Some(if pt_enabled {
            "Bitstream Passthrough: ON (Exclusive)".to_string()
        } else {
            "Bitstream Passthrough: OFF (Shared Mode)".to_string()
        });
    }
}

pub(crate) fn matches_visualizer_spec(vis: &str) -> Option<usize> {
    let vis_lower = vis.trim().to_lowercase();
    if vis_lower.is_empty() {
        return None;
    }
    let normalize = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let vis_norm = normalize(&vis_lower);
    crate::state::VISUALIZERS
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
                || (!vis_norm.is_empty() && (name_norm == vis_norm || short_norm == vis_norm))
                || (v.id == 5
                    && matches!(
                        vis_norm.as_str(),
                        "retrofire"
                            | "retro_fire"
                            | "flame"
                            | "fire"
                            | "doom"
                            | "doomfire"
                            | "doom_fire"
                            | "psxfire"
                    ))
                || (v.id == 6 && matches!(vis_norm.as_str(), "firesim" | "firesimulation"))
        })
}

fn looks_like_media_or_path_arg(arg: &std::ffi::OsStr) -> bool {
    let bytes = arg.as_encoded_bytes();
    if bytes == b"--" {
        return true;
    }
    if bytes.is_empty() || bytes[0] == b'-' {
        return false;
    }
    let Some(s) = arg.to_str() else {
        // Non-UTF-8 OS strings cannot be visualizer identifiers; treat as filesystem paths.
        return true;
    };
    if matches!(s.to_ascii_lowercase().as_str(), "list" | "help")
        || matches_visualizer_spec(s).is_some()
    {
        return false;
    }
    if s.contains('/')
        || s.contains('\\')
        || s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("file://")
    {
        return true;
    }
    let path = std::path::Path::new(s);
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && (crate::playlist::is_supported_media_extension(ext)
            || crate::playlist::is_playlist_extension(ext))
    {
        return true;
    }
    path.exists()
}

/// Normalizes bare `--bench` and bare `--vis` when immediately followed by `--` or a positional
/// file/URL argument (e.g. `rusttracker --bench song.mod` -> `--bench=300`, `rusttracker --vis song.mod`
/// -> `--vis=`) so `clap` does not swallow the media filename as the optional flag value.
fn normalize_cli_args<I>(raw_args: I) -> Vec<std::ffi::OsString>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let raw: Vec<std::ffi::OsString> = raw_args.into_iter().collect();
    if raw.is_empty() {
        return raw;
    }
    let mut out = Vec::with_capacity(raw.len());
    out.push(raw[0].clone());
    let mut passthrough_positional = false;
    let mut i = 1;
    while i < raw.len() {
        let arg = &raw[i];
        if !passthrough_positional && arg.as_encoded_bytes() == b"--" {
            passthrough_positional = true;
            out.push(arg.clone());
            i += 1;
            continue;
        }
        if !passthrough_positional && arg.as_encoded_bytes() == b"--bench" {
            let next_is_non_numeric_pos = raw.get(i + 1).is_some_and(|n| {
                let bytes = n.as_encoded_bytes();
                if bytes == b"--" {
                    return true;
                }
                !bytes.is_empty()
                    && bytes[0] != b'-'
                    && !bytes.iter().all(|b| b.is_ascii_digit())
                    && n.to_str().is_none_or(|s| s.parse::<u32>().is_err())
            });
            if next_is_non_numeric_pos {
                out.push(std::ffi::OsString::from("--bench=300"));
                i += 1;
                continue;
            }
        }
        if !passthrough_positional
            && arg.as_encoded_bytes() == b"--vis"
            && raw
                .get(i + 1)
                .is_some_and(|n| looks_like_media_or_path_arg(n))
        {
            out.push(std::ffi::OsString::from("--vis="));
            i += 1;
            continue;
        }
        out.push(arg.clone());
        i += 1;
    }
    out
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
        let crash_path = directories::ProjectDirs::from("com", "RustTracker", "RustTracker")
            .and_then(|proj| {
                let dir = proj.data_local_dir();
                std::fs::create_dir_all(dir)
                    .ok()
                    .map(|()| dir.join("rusttracker_crash.log"))
            })
            .unwrap_or_else(|| std::env::temp_dir().join("rusttracker_crash.log"));
        if std::fs::write(&crash_path, &crash_report).is_err() {
            let _ = std::fs::write(
                std::env::temp_dir().join("rusttracker_crash.log"),
                &crash_report,
            );
        }
    }));

    let filtered_args =
        std::env::args_os().filter(|arg| !arg.to_str().is_some_and(|s| s.starts_with("-psn_")));
    let mut args = Args::parse_from(normalize_cli_args(filtered_args));
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

    if args.register_steam {
        register_steam_shortcut_cli()?;
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
        state.playlist = std::sync::Arc::new(args.file.clone());
        apply_cli_audio_flags(&mut state, &args);
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
            if let Some(idx) = matches_visualizer_spec(vis) {
                state.current_visualizer_idx = idx;
                state.visualizer_mode = crate::state::VISUALIZERS[idx].id;
                state.vis_enabled[idx] = true;
                state.vis_user_overridden[idx] = true;
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
                reconcile_post_load_cli_audio_osd(&mut state, &args);
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

    audio::spawn_audio_topology_watcher(Arc::clone(&app_state), args.mic);

    let cli_upmix_override = has_cli_upmix_override(&args);
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
        if let Err(err) = run_tui(
            &mut tui.terminal,
            app_state,
            initial_stream,
            args.mic,
            cli_upmix_override,
        ) {
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

    // On Steam Deck / Gamescope (1280x800 monitor with 1280x800 desired geometry),
    // occupy the full 1280x800 display rather than subtracting desktop window frame/taskbar margins.
    if (mon_logical_w - desired_logical_w).abs() < 1.0
        && (mon_logical_h - desired_logical_h).abs() < 1.0
        && desired_logical_w == 1280.0
        && desired_logical_h == 800.0
    {
        return ((1280.0, 800.0), (1024.0, 600.0), (mon_pos_x, mon_pos_y));
    }

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

pub(crate) const MIN_PANEL_SPLIT_RATIO: f32 = 0.15;
pub(crate) const MAX_PANEL_SPLIT_RATIO: f32 = 0.85;
pub(crate) const MIN_PANEL_HEIGHT_PX: f32 = 120.0;

pub(crate) fn clamp_panel_split_ratio(ratio: f32, total_h: f32) -> f32 {
    let clamped = ratio.clamp(MIN_PANEL_SPLIT_RATIO, MAX_PANEL_SPLIT_RATIO);
    if total_h > 0.0 {
        let min_r = (MIN_PANEL_HEIGHT_PX / total_h).clamp(0.0, 1.0);
        let max_r = ((total_h - MIN_PANEL_HEIGHT_PX) / total_h).clamp(0.0, 1.0);
        clamped.clamp(min_r.min(max_r), min_r.max(max_r))
    } else {
        clamped
    }
}

pub(crate) fn apply_gamepad_stick_panel_resize(
    state: &mut AppState,
    left_stick_y: f32,
    right_stick_y: f32,
    dt: f32,
) -> bool {
    if state.show_vis_picker || state.is_file_picker_open || state.is_url_dialog_open {
        return false;
    }
    let stick_y = if left_stick_y.abs() >= right_stick_y.abs() {
        left_stick_y
    } else {
        right_stick_y
    };
    if stick_y.abs() > 0.1 {
        let scaled_delta = stick_y * dt * 1.0; // Subtract moving UP (positive Y stick)
        state.panel_split_ratio = (state.panel_split_ratio - scaled_delta)
            .clamp(MIN_PANEL_SPLIT_RATIO, MAX_PANEL_SPLIT_RATIO);
        if state.file_loaded {
            if !state.show_hud {
                state.show_hud = true;
            }
            if state.video_mode == 3 {
                state.video_mode = 0;
            }
        }
        true
    } else {
        false
    }
}

fn clear_spectrum_history_in_place(state: &mut AppState) {
    if state.spectrum_history.len() == 120
        && state.spectrum_history.iter().all(|row| row.len() == 1024)
    {
        for row in &mut state.spectrum_history {
            row.fill(0.0);
        }
    } else {
        state.spectrum_history.clear();
        for _ in 0..120 {
            state.spectrum_history.push_back(vec![0.0; 1024]);
        }
    }
}

pub(crate) fn apply_gamepad_trigger_scrub(state: &mut AppState, lt: f32, rt: f32, dt: f32) -> bool {
    if !state.file_loaded
        || state.duration_seconds <= 0.0
        || state.show_vis_picker
        || state.is_file_picker_open
        || state.is_url_dialog_open
    {
        return false;
    }
    let scrub_delta = rt - lt;
    if scrub_delta.abs() > 0.05 {
        let scrub_amount =
            (scrub_delta * dt) as f64 * (state.duration_seconds * 0.1).clamp(2.0, 30.0);
        let target = (state.current_seconds + scrub_amount).clamp(0.0, state.duration_seconds);
        state.seek_request = Some(target);
        state.current_seconds = target;
        state.track_ended = false;
        clear_spectrum_history_in_place(state);

        let sign = if scrub_delta > 0.0 { "+" } else { "" };
        state.osd_text = Some(format!("Scrubbing {}{:.1}x", sign, scrub_delta * 10.0));
        state.osd_timer = 0.5;
        true
    } else {
        false
    }
}

pub(crate) fn apply_gamepad_playback_button(state: &mut AppState, button: gilrs::Button) -> bool {
    match button {
        gilrs::Button::LeftTrigger => {
            // L1 Bumper: Previous Track (or -60s seek)
            if !state.playlist.is_empty() && state.playlist.len() > 1 && state.playlist_index > 0 {
                state.playlist_index -= 1;
                state.load_request = Some(state.playlist[state.playlist_index].clone());
                state.is_paused = false;
                state.osd_text = Some("Previous Track".to_string());
                state.osd_timer = 2.0;
                true
            } else if state.file_loaded && state.duration_seconds > 0.0 {
                let target = (state.current_seconds - 60.0).max(0.0);
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                clear_spectrum_history_in_place(state);
                state.record_seek_osd(-60.0);
                true
            } else {
                false
            }
        }
        gilrs::Button::RightTrigger => {
            // R1 Bumper: Next Track (or +60s seek)
            if !state.playlist.is_empty()
                && state.playlist.len() > 1
                && state.playlist_index + 1 < state.playlist.len()
            {
                state.playlist_index += 1;
                state.load_request = Some(state.playlist[state.playlist_index].clone());
                state.is_paused = false;
                state.osd_text = Some("Next Track".to_string());
                state.osd_timer = 2.0;
                true
            } else if state.file_loaded && state.duration_seconds > 0.0 {
                let target = (state.current_seconds + 60.0).clamp(0.0, state.duration_seconds);
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                clear_spectrum_history_in_place(state);
                state.record_seek_osd(60.0);
                true
            } else {
                false
            }
        }
        gilrs::Button::LeftThumb => {
            // L3 thumbstick click -> Toggle HUD Visibility (no collision with L2/R2 analog scrubbing)
            state.show_hud = !state.show_hud;
            state.video_mode = 0;
            state.osd_text = Some(format!(
                "HUD: {}",
                if state.show_hud { "Visible" } else { "Hidden" }
            ));
            state.osd_timer = 2.0;
            true
        }
        gilrs::Button::RightThumb => {
            // R3 thumbstick click -> Toggle Auto Surround & Height Upmix
            state.toggle_surround_upmix();
            true
        }
        gilrs::Button::North => {
            // 'Y' or Triangle -> Open File
            state.open_file_request = true;
            true
        }
        gilrs::Button::West => {
            // 'X' or Square -> Toggle Video / View Mode (or rotation in Visualizer Picker)
            if state.show_vis_picker {
                let cursor = state.vis_picker_cursor;
                if cursor != 0 && cursor < state.vis_enabled.len() {
                    state.vis_enabled[cursor] = !state.vis_enabled[cursor];
                    state.vis_user_overridden[cursor] = true;
                }
            } else if state.video_frame_rx.is_some() || state.has_video_stream {
                state.video_mode = (state.video_mode + 1) % 4;
                let mode_name = match state.video_mode {
                    0 => "Standard View",
                    1 => "Video in Track Info",
                    2 => "Video in Top Panel",
                    3 => "Full Screen Video",
                    _ => "Video",
                };
                state.osd_text = Some(format!("View: {}", mode_name));
                state.osd_timer = 2.0;
            } else {
                state.show_hud = !state.show_hud;
                state.video_mode = 0;
                let mode_name = if state.show_hud {
                    "Standard View"
                } else {
                    "Full Screen Visualizer"
                };
                state.osd_text = Some(format!("View: {}", mode_name));
                state.osd_timer = 2.0;
            }
            true
        }
        gilrs::Button::East => {
            // 'B' or Circle -> Close Visualizer Picker or Toggle Stats
            if state.show_vis_picker {
                state.show_vis_picker = false;
            } else {
                state.show_stats = !state.show_stats;
            }
            true
        }
        gilrs::Button::DPadRight => {
            if !state.show_vis_picker && state.file_loaded && state.duration_seconds > 0.0 {
                let target = (state.current_seconds + 10.0).clamp(0.0, state.duration_seconds);
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                clear_spectrum_history_in_place(state);
                state.record_seek_osd(10.0);
                true
            } else {
                false
            }
        }
        gilrs::Button::DPadLeft => {
            if !state.show_vis_picker && state.file_loaded && state.duration_seconds > 0.0 {
                let target = (state.current_seconds - 10.0).max(0.0);
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                clear_spectrum_history_in_place(state);
                state.record_seek_osd(-10.0);
                true
            } else {
                false
            }
        }
        gilrs::Button::DPadUp => {
            if state.show_vis_picker {
                if state.vis_picker_cursor == 0 {
                    state.vis_picker_cursor = crate::state::VISUALIZERS.len() - 1;
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
                state.osd_text = Some(crate::state::VISUALIZERS[idx].name.to_string());
                state.osd_timer = 2.0;
            }
            true
        }
        gilrs::Button::DPadDown => {
            if state.show_vis_picker {
                state.vis_picker_cursor =
                    (state.vis_picker_cursor + 1) % crate::state::VISUALIZERS.len();
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
                state.osd_text = Some(crate::state::VISUALIZERS[idx].name.to_string());
                state.osd_timer = 2.0;
            }
            true
        }
        gilrs::Button::South => {
            if state.show_vis_picker {
                let cursor = state.vis_picker_cursor;
                if cursor < state.vis_user_overridden.len() {
                    state.vis_user_overridden[cursor] = true;
                }
                state.current_visualizer_idx = cursor;
                state.visualizer_mode = crate::state::VISUALIZERS[cursor].id;
                state.osd_text = Some(crate::state::VISUALIZERS[cursor].name.to_string());
                state.osd_timer = 2.0;
                state.show_vis_picker = false;
            } else if state.track_ended
                || (state.current_seconds >= state.duration_seconds - 0.1
                    && state.duration_seconds > 0.0)
            {
                state.track_ended = false;
                let _ = state.rt_control.take_track_ended();
                state.seek_request = Some(0.0);
                clear_spectrum_history_in_place(state);
                state.is_paused = false;
            } else if state.playlist_index >= state.playlist.len() && !state.playlist.is_empty() {
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
            state.sync_rt_control();
            true
        }
        // Note: LeftTrigger2 (L2) and RightTrigger2 (R2) are strictly analog timeline scrubbers
        // handled in `apply_gamepad_trigger_scrub` so pulling L2 never toggles the HUD mid-scrub.
        _ => false,
    }
}

/// Parses a binary Steam `shortcuts.vdf` buffer and returns all unsigned 32-bit `appid`s
/// for shortcut entries whose `AppName` or `Exe` references `rusttracker` (case-insensitive).
pub(crate) fn extract_rusttracker_shortcut_appids(vdf_bytes: &[u8]) -> Vec<u32> {
    let mut appids = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut current_appid: Option<u32> = None;
    let mut is_rusttracker = false;

    let read_cstr = |buf: &[u8], pos: &mut usize| -> Option<String> {
        let start = *pos;
        let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
        *pos = start + rel_end + 1;
        Some(String::from_utf8_lossy(&buf[start..start + rel_end]).into_owned())
    };

    while i < vdf_bytes.len() {
        let tag = vdf_bytes[i];
        i += 1;
        match tag {
            0x00 => {
                // Start of nested object
                if read_cstr(vdf_bytes, &mut i).is_none() {
                    break;
                }
                depth += 1;
                if depth == 2 {
                    current_appid = None;
                    is_rusttracker = false;
                }
            }
            0x01 => {
                // String property
                let Some(key) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                let Some(val) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                if depth == 2
                    && (key.eq_ignore_ascii_case("AppName") || key.eq_ignore_ascii_case("Exe"))
                    && val.to_ascii_lowercase().contains("rusttracker")
                {
                    is_rusttracker = true;
                }
            }
            0x02 => {
                // 32-bit integer property
                let Some(key) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                let Some(raw) = vdf_bytes.get(i..i + 4) else {
                    break;
                };
                i += 4;
                if depth == 2 && key.eq_ignore_ascii_case("appid") {
                    current_appid = Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]));
                }
            }
            0x08 => {
                // End of nested object
                if depth == 2 {
                    if is_rusttracker
                        && let Some(id) = current_appid
                        && !appids.contains(&id)
                    {
                        appids.push(id);
                    }
                    current_appid = None;
                    is_rusttracker = false;
                }
                depth = depth.saturating_sub(1);
            }
            _ => break,
        }
    }
    appids
}

/// Normalizes any RustTracker shortcut entry in a binary `shortcuts.vdf` buffer so that
/// filename-based `AppName` values (e.g. `"RustTracker-SteamDeck-GamePad.AppImage"`) become
/// `"RustTracker"` and an empty `icon` field is populated with `default_icon_path`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn normalize_rusttracker_shortcuts_vdf(
    vdf_bytes: &[u8],
    default_icon_path: Option<&str>,
) -> Option<Vec<u8>> {
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut is_rusttracker = false;
    let mut app_name_span: Option<(usize, usize, String)> = None;
    let mut icon_span: Option<(usize, usize, String)> = None;

    let read_cstr_span = |buf: &[u8], pos: &mut usize| -> Option<(usize, usize, String)> {
        let start = *pos;
        let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
        let end = start + rel_end;
        *pos = end + 1;
        Some((
            start,
            end,
            String::from_utf8_lossy(&buf[start..end]).into_owned(),
        ))
    };

    while i < vdf_bytes.len() {
        let tag = vdf_bytes[i];
        i += 1;
        match tag {
            0x00 => {
                if read_cstr_span(vdf_bytes, &mut i).is_none() {
                    break;
                }
                depth += 1;
                if depth == 2 {
                    is_rusttracker = false;
                    app_name_span = None;
                    icon_span = None;
                }
            }
            0x01 => {
                let Some((_, _, key)) = read_cstr_span(vdf_bytes, &mut i) else {
                    break;
                };
                let Some((val_start, val_end, val)) = read_cstr_span(vdf_bytes, &mut i) else {
                    break;
                };
                if depth == 2 {
                    if (key.eq_ignore_ascii_case("AppName") || key.eq_ignore_ascii_case("Exe"))
                        && val.to_ascii_lowercase().contains("rusttracker")
                    {
                        is_rusttracker = true;
                    }
                    if key.eq_ignore_ascii_case("AppName") {
                        app_name_span = Some((val_start, val_end, val));
                    } else if key.eq_ignore_ascii_case("icon") {
                        icon_span = Some((val_start, val_end, val));
                    }
                }
            }
            0x02 => {
                if read_cstr_span(vdf_bytes, &mut i).is_none() {
                    break;
                };
                if vdf_bytes.get(i..i + 4).is_none() {
                    break;
                }
                i += 4;
            }
            0x08 => {
                if depth == 2 {
                    if is_rusttracker {
                        if let Some((start, end, name)) = app_name_span.take() {
                            let lower = name.to_ascii_lowercase();
                            if name != "RustTracker"
                                && (lower.ends_with(".appimage")
                                    || lower.starts_with("rusttracker-")
                                    || lower.starts_with("rusttracker_")
                                    || lower == "rusttracker")
                            {
                                replacements.push((start, end, "RustTracker".to_string()));
                            }
                        }
                        if let Some((start, end, icon)) = icon_span.take()
                            && icon.is_empty()
                            && let Some(icon_path) = default_icon_path
                            && !icon_path.is_empty()
                        {
                            replacements.push((start, end, icon_path.to_string()));
                        }
                    }
                    is_rusttracker = false;
                    app_name_span = None;
                    icon_span = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => break,
        }
    }

    if replacements.is_empty() {
        return None;
    }

    replacements.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut out = vdf_bytes.to_vec();
    for (start, end, new_val) in replacements {
        out.splice(start..end, new_val.into_bytes());
    }
    Some(out)
}

/// Inserts or updates a RustTracker shortcut entry in a binary `shortcuts.vdf` buffer,
/// returning the updated buffer and the shortcut's unsigned 32-bit `appid`.
pub(crate) fn upsert_rusttracker_shortcut_in_vdf(
    vdf_bytes: &[u8],
    exe_path: &str,
    start_dir: &str,
    icon_path: &str,
) -> (Vec<u8>, u32) {
    let existing_ids = extract_rusttracker_shortcut_appids(vdf_bytes);
    if let Some(&appid) = existing_ids.first() {
        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        let mut i = 0usize;
        let mut depth = 0usize;
        let mut is_rusttracker = false;
        let mut app_name_span: Option<(usize, usize)> = None;
        let mut exe_span: Option<(usize, usize)> = None;
        let mut start_dir_span: Option<(usize, usize)> = None;
        let mut icon_span: Option<(usize, usize)> = None;

        let read_cstr_span = |buf: &[u8], pos: &mut usize| -> Option<(usize, usize, String)> {
            let start = *pos;
            let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
            let end = start + rel_end;
            *pos = end + 1;
            Some((
                start,
                end,
                String::from_utf8_lossy(&buf[start..end]).into_owned(),
            ))
        };

        while i < vdf_bytes.len() {
            let tag = vdf_bytes[i];
            i += 1;
            match tag {
                0x00 => {
                    if read_cstr_span(vdf_bytes, &mut i).is_none() {
                        break;
                    }
                    depth += 1;
                    if depth == 2 {
                        is_rusttracker = false;
                        app_name_span = None;
                        exe_span = None;
                        start_dir_span = None;
                        icon_span = None;
                    }
                }
                0x01 => {
                    let Some((_, _, key)) = read_cstr_span(vdf_bytes, &mut i) else {
                        break;
                    };
                    let Some((val_start, val_end, val)) = read_cstr_span(vdf_bytes, &mut i) else {
                        break;
                    };
                    if depth == 2 {
                        if (key.eq_ignore_ascii_case("AppName") || key.eq_ignore_ascii_case("Exe"))
                            && val.to_ascii_lowercase().contains("rusttracker")
                        {
                            is_rusttracker = true;
                        }
                        if key.eq_ignore_ascii_case("AppName") {
                            app_name_span = Some((val_start, val_end));
                        } else if key.eq_ignore_ascii_case("Exe") {
                            exe_span = Some((val_start, val_end));
                        } else if key.eq_ignore_ascii_case("StartDir") {
                            start_dir_span = Some((val_start, val_end));
                        } else if key.eq_ignore_ascii_case("icon") {
                            icon_span = Some((val_start, val_end));
                        }
                    }
                }
                0x02 => {
                    if read_cstr_span(vdf_bytes, &mut i).is_none() {
                        break;
                    }
                    if vdf_bytes.get(i..i + 4).is_none() {
                        break;
                    }
                    i += 4;
                }
                0x08 => {
                    if depth == 2 {
                        if is_rusttracker {
                            if let Some((s, e)) = app_name_span.take() {
                                replacements.push((s, e, "RustTracker".to_string()));
                            }
                            if !exe_path.is_empty()
                                && let Some((s, e)) = exe_span.take()
                            {
                                replacements.push((s, e, format!("\"{exe_path}\"")));
                            }
                            if !start_dir.is_empty()
                                && let Some((s, e)) = start_dir_span.take()
                            {
                                replacements.push((s, e, start_dir.to_string()));
                            }
                            if !icon_path.is_empty()
                                && let Some((s, e)) = icon_span.take()
                            {
                                replacements.push((s, e, icon_path.to_string()));
                            }
                        }
                        is_rusttracker = false;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => break,
            }
        }
        replacements.sort_by_key(|a| std::cmp::Reverse(a.0));
        let mut out = vdf_bytes.to_vec();
        for (start, end, new_val) in replacements {
            out.splice(start..end, new_val.into_bytes());
        }
        return (out, appid);
    }

    // Create a new shortcut entry
    let appid: u32 = 3929032336; // 0xEA304690
    let mut next_idx = 0usize;
    let mut i = 0usize;
    let mut depth = 0usize;
    let read_cstr = |buf: &[u8], pos: &mut usize| -> Option<String> {
        let start = *pos;
        let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
        *pos = start + rel_end + 1;
        Some(String::from_utf8_lossy(&buf[start..start + rel_end]).into_owned())
    };
    while i < vdf_bytes.len() {
        let tag = vdf_bytes[i];
        i += 1;
        match tag {
            0x00 => {
                let Some(key) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                depth += 1;
                if depth == 2
                    && let Ok(n) = key.parse::<usize>()
                {
                    next_idx = next_idx.max(n + 1);
                }
            }
            0x01 => {
                if read_cstr(vdf_bytes, &mut i).is_none() || read_cstr(vdf_bytes, &mut i).is_none()
                {
                    break;
                }
            }
            0x02 => {
                if read_cstr(vdf_bytes, &mut i).is_none() || vdf_bytes.get(i..i + 4).is_none() {
                    break;
                }
                i += 4;
            }
            0x08 => {
                depth = depth.saturating_sub(1);
            }
            _ => break,
        }
    }

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);

    let mut entry = Vec::new();
    entry.push(0x00);
    entry.extend_from_slice(next_idx.to_string().as_bytes());
    entry.push(0x00);
    entry.extend_from_slice(b"\x02appid\x00");
    entry.extend_from_slice(&appid.to_le_bytes());
    entry.extend_from_slice(b"\x01AppName\x00RustTracker\x00");
    entry.extend_from_slice(b"\x01Exe\x00\"");
    entry.extend_from_slice(exe_path.as_bytes());
    entry.extend_from_slice(b"\"\x00");
    entry.extend_from_slice(b"\x01StartDir\x00");
    entry.extend_from_slice(start_dir.as_bytes());
    entry.push(0x00);
    entry.extend_from_slice(b"\x01icon\x00");
    entry.extend_from_slice(icon_path.as_bytes());
    entry.push(0x00);
    entry.extend_from_slice(b"\x01ShortcutPath\x00\x00");
    entry.extend_from_slice(b"\x01LaunchOptions\x00\x00");
    entry.extend_from_slice(b"\x02IsHidden\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x02AllowDesktopConfig\x00\x01\x00\x00\x00");
    entry.extend_from_slice(b"\x02AllowOverlay\x00\x01\x00\x00\x00");
    entry.extend_from_slice(b"\x02OpenVR\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x02Devkit\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x01DevkitGameID\x00\x00");
    entry.extend_from_slice(b"\x02DevkitOverrideAppID\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x02LastPlayTime\x00");
    entry.extend_from_slice(&now_epoch.to_le_bytes());
    entry.extend_from_slice(b"\x01FlatpakAppID\x00\x00");
    entry.extend_from_slice(b"\x01sortas\x00\x00");
    entry.extend_from_slice(b"\x00tags\x00\x08\x08");

    let mut out = if vdf_bytes.starts_with(b"\x00shortcuts\x00") && vdf_bytes.len() >= 13 {
        let mut trimmed = vdf_bytes.to_vec();
        while trimmed.last() == Some(&0x08) {
            trimmed.pop();
        }
        trimmed
    } else {
        b"\x00shortcuts\x00".to_vec()
    };
    out.extend_from_slice(&entry);
    out.extend_from_slice(b"\x08\x08");
    (out, appid)
}

fn register_steam_shortcut_cli() -> Result<(), Box<dyn Error>> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Err("HOME environment variable is not set".into());
    };
    let exe_path = std::env::var("APPIMAGE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "/home/deck/Downloads/RustTracker-SteamDeck.AppImage".to_string());
    let start_dir = std::path::Path::new(&exe_path)
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| format!("{}/", s.trim_end_matches('/')))
        .unwrap_or_else(|| "/home/deck/Downloads/".to_string());
    let icon_path = home.join(".local/share/icons/hicolor/512x512/apps/rusttracker.png");
    let icon_path_str = icon_path.to_string_lossy().into_owned();

    // Stop steam-launcher.service if running on SteamOS so Steam flushes shortcuts.vdf first
    let steam_service_was_active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "steam-launcher.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if steam_service_was_active {
        println!("[steam] Stopping steam-launcher.service to safely update shortcuts.vdf...");
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "stop", "steam-launcher.service"])
            .status();
    }

    let mut total_registered = 0usize;
    for rel in [".local/share/Steam", ".steam/steam"] {
        let root = home.join(rel);
        let userdata = root.join("userdata");
        if !userdata.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&userdata) {
            for entry in entries.flatten() {
                let user_dir = entry.path();
                if !user_dir.is_dir() {
                    continue;
                }
                let config_dir = user_dir.join("config");
                let _ = std::fs::create_dir_all(&config_dir);
                let vdf_path = config_dir.join("shortcuts.vdf");
                let existing = std::fs::read(&vdf_path).unwrap_or_default();
                let (updated, appid) = upsert_rusttracker_shortcut_in_vdf(
                    &existing,
                    &exe_path,
                    &start_dir,
                    &icon_path_str,
                );
                std::fs::write(&vdf_path, updated)?;
                println!(
                    "[steam] Registered 'RustTracker' (AppID {}) in {}",
                    appid,
                    vdf_path.display()
                );
                total_registered += 1;
            }
        }
        if total_registered > 0 {
            break;
        }
    }

    auto_provision_steam_artwork();

    if steam_service_was_active {
        println!("[steam] Restarting steam-launcher.service...");
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "start", "steam-launcher.service"])
            .status();
    }

    println!(
        "[steam] Successfully registered RustTracker and provisioned SteamOS Home Screen artwork ({} user profile(s)).",
        total_registered
    );
    Ok(())
}

/// Copies Steam Grid (`userdata/<id>/config/grid/`) and LibraryCache (`appcache/librarycache/`)
/// artwork for all discovered RustTracker non-Steam shortcuts under `steam_root`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn provision_steam_shortcut_artwork_in_root(
    steam_root: &std::path::Path,
    steamgrid_dir: &std::path::Path,
    icon_png: Option<&std::path::Path>,
) -> usize {
    let userdata_dir = steam_root.join("userdata");
    let Ok(entries) = std::fs::read_dir(&userdata_dir) else {
        return 0;
    };
    let copy_if_missing = |src: &std::path::Path, dst: &std::path::Path| -> bool {
        if !dst.exists() && src.exists() {
            std::fs::copy(src, dst).is_ok()
        } else {
            false
        }
    };

    let header_src = steamgrid_dir.join("header.jpg");
    let capsule_src = steamgrid_dir.join("library_600x900.jpg");
    let hero_src = steamgrid_dir.join("library_hero.jpg");
    let hero_blur_src = steamgrid_dir.join("library_hero_blur.jpg");
    let logo_src = steamgrid_dir.join("logo.png");
    let icon_jpg_src = steamgrid_dir.join("icon.jpg");

    let libcache_dir = steam_root.join("appcache").join("librarycache");
    let mut provisioned_shortcuts = 0usize;

    for entry in entries.flatten() {
        let user_dir = entry.path();
        if !user_dir.is_dir() {
            continue;
        }
        let vdf_path = user_dir.join("config").join("shortcuts.vdf");
        let Ok(vdf_bytes) = std::fs::read(&vdf_path) else {
            continue;
        };
        let appids = extract_rusttracker_shortcut_appids(&vdf_bytes);
        if appids.is_empty() {
            continue;
        }
        let icon_path_str = icon_png.and_then(|p| p.to_str());
        if let Some(updated_vdf) = normalize_rusttracker_shortcuts_vdf(&vdf_bytes, icon_path_str) {
            let _ = std::fs::write(&vdf_path, updated_vdf);
        }
        let grid_dir = user_dir.join("config").join("grid");
        let _ = std::fs::create_dir_all(&grid_dir);
        let _ = std::fs::create_dir_all(&libcache_dir);

        for appid in appids {
            provisioned_shortcuts += 1;
            let _ = copy_if_missing(&header_src, &grid_dir.join(format!("{appid}.jpg")));
            let _ = copy_if_missing(&capsule_src, &grid_dir.join(format!("{appid}p.jpg")));
            let _ = copy_if_missing(&hero_src, &grid_dir.join(format!("{appid}_hero.jpg")));
            let _ = copy_if_missing(&logo_src, &grid_dir.join(format!("{appid}_logo.png")));
            let _ = copy_if_missing(&icon_jpg_src, &grid_dir.join(format!("{appid}_icon.jpg")));
            if let Some(png) = icon_png {
                let _ = copy_if_missing(png, &grid_dir.join(format!("{appid}_icon.png")));
            }

            let app_cache_sub = libcache_dir.join(appid.to_string());
            let _ = std::fs::create_dir_all(&app_cache_sub);
            let _ = copy_if_missing(&header_src, &app_cache_sub.join("header.jpg"));
            let _ = copy_if_missing(&header_src, &app_cache_sub.join("library_header.jpg"));
            let _ = copy_if_missing(&capsule_src, &app_cache_sub.join("library_600x900.jpg"));
            let _ = copy_if_missing(&hero_src, &app_cache_sub.join("library_hero.jpg"));
            let _ = copy_if_missing(&hero_blur_src, &app_cache_sub.join("library_hero_blur.jpg"));
            let _ = copy_if_missing(&logo_src, &app_cache_sub.join("logo.png"));
            let _ = copy_if_missing(&icon_jpg_src, &app_cache_sub.join("icon.jpg"));

            let _ = copy_if_missing(
                &header_src,
                &libcache_dir.join(format!("{appid}_header.jpg")),
            );
            let _ = copy_if_missing(
                &header_src,
                &libcache_dir.join(format!("{appid}_library_header.jpg")),
            );
            let _ = copy_if_missing(
                &capsule_src,
                &libcache_dir.join(format!("{appid}_library_600x900.jpg")),
            );
            let _ = copy_if_missing(
                &hero_src,
                &libcache_dir.join(format!("{appid}_library_hero.jpg")),
            );
            let _ = copy_if_missing(
                &hero_blur_src,
                &libcache_dir.join(format!("{appid}_library_hero_blur.jpg")),
            );
            let _ = copy_if_missing(&logo_src, &libcache_dir.join(format!("{appid}_logo.png")));
            let _ = copy_if_missing(
                &icon_jpg_src,
                &libcache_dir.join(format!("{appid}_icon.jpg")),
            );
        }
    }
    provisioned_shortcuts
}

fn auto_provision_steam_artwork() {
    #[cfg(target_os = "linux")]
    {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            return;
        };
        let mut steamgrid_candidates = Vec::new();
        let mut icon_candidates = Vec::new();
        if let Some(appdir) = std::env::var_os("RUSTTRACKER_APPDIR")
            .or_else(|| std::env::var_os("APPDIR"))
            .map(std::path::PathBuf::from)
        {
            steamgrid_candidates.push(appdir.join("usr/share/rusttracker/steamgrid"));
            icon_candidates
                .push(appdir.join("usr/share/icons/hicolor/512x512/apps/rusttracker.png"));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            steamgrid_candidates.push(exe_dir.join("../share/rusttracker/steamgrid"));
            steamgrid_candidates.push(exe_dir.join("assets/steamgrid"));
            icon_candidates
                .push(exe_dir.join("../share/icons/hicolor/512x512/apps/rusttracker.png"));
            icon_candidates.push(exe_dir.join("assets/icons/hicolor/512x512/apps/rusttracker.png"));
        }
        steamgrid_candidates.push(std::path::PathBuf::from("assets/steamgrid"));
        icon_candidates.push(std::path::PathBuf::from(
            "assets/icons/hicolor/512x512/apps/rusttracker.png",
        ));

        let Some(steamgrid_dir) = steamgrid_candidates
            .into_iter()
            .find(|p| p.join("header.jpg").exists())
        else {
            return;
        };
        let bundled_icon_png = icon_candidates.into_iter().find(|p| p.exists());
        let persistent_icon_path =
            home.join(".local/share/icons/hicolor/512x512/apps/rusttracker.png");
        if !persistent_icon_path.exists()
            && let Some(ref src_icon) = bundled_icon_png
        {
            if let Some(parent) = persistent_icon_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(src_icon, &persistent_icon_path);
        }
        let icon_png = if persistent_icon_path.exists() {
            Some(persistent_icon_path)
        } else {
            bundled_icon_png
        };

        for rel in [".local/share/Steam", ".steam/steam"] {
            let root = home.join(rel);
            if root.join("userdata").is_dir() {
                let _ = provision_steam_shortcut_artwork_in_root(
                    &root,
                    &steamgrid_dir,
                    icon_png.as_deref(),
                );
            }
        }
    }
}

#[inline]
fn effective_window_scale(window: &winit::window::Window, force_unit_scale: bool) -> f64 {
    if force_unit_scale {
        1.0
    } else {
        window.scale_factor().max(1.0)
    }
}

fn compute_initial_window_geometry(
    monitor: Option<winit::monitor::MonitorHandle>,
    desired_logical_w: f64,
    desired_logical_h: f64,
    force_unit_scale: bool,
) -> (
    winit::dpi::Size,
    winit::dpi::Size,
    Option<winit::dpi::PhysicalPosition<i32>>,
) {
    let to_size = |w: f64, h: f64| -> winit::dpi::Size {
        if force_unit_scale {
            winit::dpi::PhysicalSize::new(w.round().max(1.0) as u32, h.round().max(1.0) as u32)
                .into()
        } else {
            winit::dpi::LogicalSize::new(w, h).into()
        }
    };
    if let Some(mon) = monitor {
        let mon_phys = mon.size();
        if mon_phys.width > 0 && mon_phys.height > 0 {
            let mon_pos = mon.position();
            let scale = if force_unit_scale {
                1.0
            } else {
                mon.scale_factor()
            };
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
                to_size(target_w, target_h),
                to_size(min_w, min_h),
                Some(winit::dpi::PhysicalPosition::new(pos_x, pos_y)),
            );
        }
    }
    (
        to_size(desired_logical_w, desired_logical_h),
        to_size(
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
    force_unit_scale: bool,
) {
    let Some(mon) = window
        .current_monitor()
        .or_else(|| window.primary_monitor())
        .or_else(|| window.available_monitors().next())
    else {
        return;
    };

    let scale = effective_window_scale(window, force_unit_scale);
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
    let is_deck_full_display = desired_logical_w == Some(1280.0)
        && desired_logical_h == Some(800.0)
        && (mon_logical_w - 1280.0).abs() < 1.0
        && (mon_logical_h - 800.0).abs() < 1.0;
    if is_deck_full_display {
        let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(
            mon_phys.width,
            mon_phys.height,
        ));
        window.set_outer_position(winit::dpi::PhysicalPosition::new(mon_pos.x, mon_pos.y));
        return;
    }
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
    let min_size: winit::dpi::Size = if force_unit_scale {
        winit::dpi::PhysicalSize::new(
            min_logical_w.round().max(1.0) as u32,
            min_logical_h.round().max(1.0) as u32,
        )
        .into()
    } else {
        winit::dpi::LogicalSize::new(min_logical_w, min_logical_h).into()
    };
    window.set_min_inner_size(Some(min_size));

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

/// Queries the live cursor position in logical points relative to the window's client area.
/// During external OS file drag-and-drop, winit 0.30 does not emit `WindowEvent::CursorMoved`
/// (and `CursorLeft` clears egui's pointer state when leaving the window to grab a file),
/// so we query the windowing system directly during `HoveredFile` / `DroppedFile`.
fn query_window_cursor_pos(
    window: &winit::window::Window,
    force_unit_scale: bool,
) -> Option<egui::Pos2> {
    #[cfg(target_os = "linux")]
    {
        use winit::raw_window_handle::{
            HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
        };
        let dh = window.display_handle().ok()?.as_raw();
        let wh = window.window_handle().ok()?.as_raw();
        let (display_ptr, xwindow) = match (dh, wh) {
            (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(w)) => {
                (d.display?.as_ptr(), w.window)
            }
            _ => return None,
        };
        type XQueryPointerFn = unsafe extern "C" fn(
            display: *mut std::ffi::c_void,
            w: std::ffi::c_ulong,
            root_return: *mut std::ffi::c_ulong,
            child_return: *mut std::ffi::c_ulong,
            root_x_return: *mut std::ffi::c_int,
            root_y_return: *mut std::ffi::c_int,
            win_x_return: *mut std::ffi::c_int,
            win_y_return: *mut std::ffi::c_int,
            mask_return: *mut std::ffi::c_uint,
        ) -> std::ffi::c_int;

        static XQUERY_POINTER: std::sync::OnceLock<Option<XQueryPointerFn>> =
            std::sync::OnceLock::new();
        let func = (*XQUERY_POINTER.get_or_init(|| unsafe {
            unsafe extern "C" {
                fn dlopen(
                    filename: *const std::ffi::c_char,
                    flags: std::ffi::c_int,
                ) -> *mut std::ffi::c_void;
                fn dlsym(
                    handle: *mut std::ffi::c_void,
                    symbol: *const std::ffi::c_char,
                ) -> *mut std::ffi::c_void;
            }
            // RTLD_LAZY = 1
            let lib = dlopen(c"libX11.so.6".as_ptr(), 1);
            if lib.is_null() {
                return None;
            }
            let sym = dlsym(lib, c"XQueryPointer".as_ptr());
            if sym.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*mut std::ffi::c_void, XQueryPointerFn>(sym))
            }
        }))?;

        let mut root_ret: std::ffi::c_ulong = 0;
        let mut child_ret: std::ffi::c_ulong = 0;
        let mut root_x: std::ffi::c_int = 0;
        let mut root_y: std::ffi::c_int = 0;
        let mut win_x: std::ffi::c_int = 0;
        let mut win_y: std::ffi::c_int = 0;
        let mut mask_ret: std::ffi::c_uint = 0;
        let ok = unsafe {
            func(
                display_ptr,
                xwindow,
                &mut root_ret,
                &mut child_ret,
                &mut root_x,
                &mut root_y,
                &mut win_x,
                &mut win_y,
                &mut mask_ret,
            )
        };
        if ok != 0 {
            let sf = effective_window_scale(window, force_unit_scale) as f32;
            Some(egui::pos2(win_x as f32 / sf, win_y as f32 / sf))
        } else {
            None
        }
    }
    #[cfg(windows)]
    {
        let _ = force_unit_scale;
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetCursorPos(lpPoint: *mut [i32; 2]) -> i32;
        }
        let mut pt = [0i32; 2];
        if unsafe { GetCursorPos(&mut pt) } != 0
            && let Ok(inner_pos) = window.inner_position()
        {
            let sf = window.scale_factor().max(1.0) as f32;
            Some(egui::pos2(
                (pt[0] - inner_pos.x) as f32 / sf,
                (pt[1] - inner_pos.y) as f32 / sf,
            ))
        } else {
            None
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = force_unit_scale;
        use objc2::encode::{Encode, Encoding};
        use objc2::msg_send;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        #[repr(C)]
        #[derive(Copy, Clone)]
        struct NSPoint {
            x: f64,
            y: f64,
        }
        unsafe impl Encode for NSPoint {
            const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
        }

        let wh = window.window_handle().ok()?.as_raw();
        if let RawWindowHandle::AppKit(handle) = wh {
            let ns_view = handle.ns_view.as_ptr() as *mut objc2::runtime::AnyObject;
            if !ns_view.is_null() {
                unsafe {
                    let ns_window: *mut objc2::runtime::AnyObject = msg_send![ns_view, window];
                    if !ns_window.is_null() {
                        let win_pt: NSPoint =
                            msg_send![ns_window, mouseLocationOutsideOfEventStream];
                        let null_view: *const objc2::runtime::AnyObject = std::ptr::null();
                        let view_pt: NSPoint =
                            msg_send![ns_view, convertPoint: win_pt, fromView: null_view];
                        return Some(egui::pos2(view_pt.x as f32, view_pt.y as f32));
                    }
                }
            }
        }
        None
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = (window, force_unit_scale);
        None
    }
}

async fn run_gui(
    args: Args,
    app_state: Arc<Mutex<AppState>>,
    mut active_stream: Option<audio::PlaybackHandle>,
    ipc_rx: crossbeam_channel::Receiver<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    auto_provision_steam_artwork();
    let is_steam_deck = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
        .map(|s| s.trim().to_lowercase().contains("valve"))
        .unwrap_or(false)
        || std::env::var("SteamDeck").is_ok()
        || std::env::var("SteamOS").is_ok();
    let is_gamescope = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .eq_ignore_ascii_case("gamescope")
        || std::env::var("XDG_SESSION_DESKTOP")
            .unwrap_or_default()
            .eq_ignore_ascii_case("gamescope");
    let is_game_mode = is_gamescope || std::env::var("STEAM_DECK").is_ok() || is_steam_deck;

    // Gamescope's XWayland server advertises a 1280x800 display with 100mm x 150mm physical
    // dimensions (~212 DPI), which causes winit's X11 backend to compute a 2.208x scale factor.
    // Instead of calling `unsafe { std::env::set_var(...) }` in a multi-threaded process (which is
    // UB under Rust 2024 when background IPC/audio threads are active), enforce a 1.0 effective
    // pixel-per-point ratio via PhysicalSize window geometry + egui zoom compensation.
    let force_unit_scale = cfg!(target_os = "linux")
        && is_game_mode
        && std::env::var("WINIT_X11_SCALE_FACTOR").is_err();

    let is_fullscreen = !args.windowed
        && (args.fullscreen || (is_gamescope && std::env::var("CAPTURE_FRAME").is_err()));
    let bench = args.bench;
    let is_mic_launch = args.mic;
    let is_uncapped = args.uncapped;
    let cli_upmix_override = has_cli_upmix_override(&args);

    if let Some(bf) = bench {
        crate::state::BENCH_FRAMES_OVERRIDE.store(bf, std::sync::atomic::Ordering::Relaxed);
    }

    let mut event_loop_builder = EventLoop::builder();
    #[cfg(target_os = "linux")]
    {
        // Default to native Wayland when WAYLAND_DISPLAY is active, falling back to
        // X11/XWayland when only DISPLAY is present, or when explicitly requested via
        // RUSTTRACKER_X11=1 or RUSTTRACKER_WAYLAND=0/false.
        if should_prefer_x11_backend(
            std::env::var("RUSTTRACKER_WAYLAND").ok().as_deref(),
            std::env::var("RUSTTRACKER_X11").ok().as_deref(),
            std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()),
            std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()),
        ) {
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

    let (desired_logical_w, desired_logical_h): (f64, f64) = if is_game_mode {
        (1280.0, 800.0)
    } else {
        (1440.0, 840.0)
    };
    let initial_size: winit::dpi::Size = if force_unit_scale {
        winit::dpi::PhysicalSize::new(
            desired_logical_w.round() as u32,
            desired_logical_h.round() as u32,
        )
        .into()
    } else {
        winit::dpi::LogicalSize::new(desired_logical_w, desired_logical_h).into()
    };
    let min_initial_size: winit::dpi::Size = if force_unit_scale {
        winit::dpi::PhysicalSize::new(1024, 600).into()
    } else {
        winit::dpi::LogicalSize::new(1024.0, 600.0).into()
    };

    let mut attrs = winit::window::Window::default_attributes()
        .with_title("RustTracker Vulkan Visualizer")
        .with_theme(Some(winit::window::Theme::Dark))
        .with_visible(false)
        .with_inner_size(initial_size)
        .with_min_inner_size(min_initial_size)
        .with_window_icon(Some(window_icon));

    if is_fullscreen {
        attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
    }

    #[cfg(target_os = "linux")]
    {
        attrs = WindowAttributesExtWayland::with_name(attrs, "rusttracker", "rusttracker");
        attrs = WindowAttributesExtX11::with_name(attrs, "rusttracker", "rusttracker");
    }

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
                let (init_size, min_size, init_pos) = compute_initial_window_geometry(
                    monitor,
                    desired_logical_w,
                    desired_logical_h,
                    force_unit_scale,
                );
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
                    force_unit_scale,
                );
            }
            let window = Arc::new(window);
            if force_unit_scale {
                egui_ctx_for_init.set_zoom_factor(1.0 / (window.scale_factor() as f32).max(0.1));
            }
            let mut engine = pollster::block_on(VulkanEngine::new(window.clone()));
            let initial_refresh_hz = window
                .current_monitor()
                .or_else(|| window.primary_monitor())
                .and_then(|m| m.refresh_rate_millihertz())
                .map(|mhz| (mhz as f32 / 1000.0).clamp(30.0, 500.0));
            if let Some(hz) = initial_refresh_hz {
                engine.smooth_dt = 1.0 / (hz as f64);
            }
            {
                let mut state = app_state_for_init.lock().unwrap();
                if let Some(hz) = initial_refresh_hz {
                    state.target_fps = hz.round() as u32;
                }
                state.apply_gpu_capability_profile(
                    &engine.adapter_info,
                    engine.config.width,
                    engine.config.height,
                );
            }
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
                    let num_channels = state.channel_vus.len();
                    if state.peak_vus.len() != num_channels {
                        state.peak_vus.resize(num_channels, 0.0);
                    }
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
                app_state_for_init.lock().unwrap().track_info_rect = engine.track_info_rect;
            }
            window.set_visible(true);
            #[cfg(windows)]
            {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(wh) = window.window_handle()
                    && let RawWindowHandle::Win32(h) = wh.as_raw()
                {
                    unsafe {
                        #[link(name = "user32")]
                        unsafe extern "system" {
                            fn ShowWindow(hwnd: isize, nCmdShow: i32) -> i32;
                            fn SetForegroundWindow(hwnd: isize) -> i32;
                        }
                        let hwnd = h.hwnd.get();
                        ShowWindow(hwnd, 1);
                        ShowWindow(hwnd, 1);
                        SetForegroundWindow(hwnd);
                    }
                }
            }
            window.focus_window();
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
    let mut touch_controller = crate::touch::TouchGestureController::new();
    let mut last_touch_scrub_seek = Instant::now();

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
        .default_size([920.0, 560.0])
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
    let mut pending_dropped_files: Vec<String> = Vec::new();
    let mut drop_cursor_pos: Option<[f32; 2]> = None;
    let mut bench_frame_count = 0u32;
    let mut frame_dt_ring = [1.0f32 / 60.0; 60];
    let mut frame_dt_idx = 0usize;
    let mut desktop_frame_count = 0u64;
    let mut frame_left_stick_y = 0.0f32;
    let mut frame_right_stick_y = 0.0f32;
    let mut frame_left_trigger = 0.0f32;
    let mut frame_right_trigger = 0.0f32;

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
                    let is_typing = {
                        let state = app_state.lock().unwrap();
                        state.is_url_dialog_open || state.is_file_picker_open
                    };

                    // Do not forward keyboard events to egui unless a text input dialog is open;
                    // otherwise clicking any HUD button leaves egui keyboard focus on that button
                    // and pressing Space/Tab/Enter activates the focused HUD button instead of
                    // running global hotkeys (like Space for Pause/Play).
                    let response = if !is_typing
                        && matches!(
                            event,
                            WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_)
                        ) {
                        egui_ctx.memory_mut(|m| {
                            if let Some(id) = m.focused() {
                                m.surrender_focus(id);
                            }
                        });
                        egui_winit::EventResponse {
                            consumed: false,
                            repaint: false,
                        }
                    } else {
                        egui_state.on_window_event(window, event)
                    };

                    if let WindowEvent::CursorMoved { position, .. } = event {
                        last_mouse_move = Instant::now();
                        let sf = effective_window_scale(window, force_unit_scale) as f32;
                        let pos = egui::pos2(position.x as f32 / sf, position.y as f32 / sf);
                        last_cursor_pos = Some(pos);
                        {
                            let mut state = app_state.lock().unwrap();
                            if state.hovered_file.is_some() {
                                state.drag_cursor_pos = Some([pos.x, pos.y]);
                            }
                        }
                        if !is_cursor_visible {
                            window.set_cursor_visible(true);
                            is_cursor_visible = true;
                        }
                    }

                    if let WindowEvent::Touch(touch) = event {
                        last_mouse_move = Instant::now();
                        let (modal_open, already_touch_scrubbing, split_ratio) = {
                            let st = app_state.lock().unwrap();
                            (
                                st.is_url_dialog_open
                                    || st.is_file_picker_open
                                    || st.show_vis_picker
                                    || st.show_help,
                                st.is_touch_scrubbing,
                                st.panel_split_ratio,
                            )
                        };
                        if modal_open
                            || ((response.consumed || egui_ctx.egui_is_using_pointer())
                                && !already_touch_scrubbing)
                        {
                            touch_controller.reset();
                            app_state.lock().unwrap().cancel_touch_scrub();
                        } else {
                            let size = window.inner_size();
                            let scale_factor =
                                (effective_window_scale(window, force_unit_scale) as f32).max(1.0);
                            touch_controller.set_scale_factor(scale_factor);
                            let is_portrait = size.width < size.height;
                            let min_h_px = 220.0 * scale_factor;
                            let split_y = (size.height as f32 * split_ratio)
                                .clamp(min_h_px, (size.height as f32 - min_h_px).max(min_h_px));
                            if let Some(gesture) = touch_controller.handle_touch(
                                touch,
                                size.width as f32,
                                size.height as f32,
                                is_portrait,
                                split_y,
                            ) {
                                match gesture {
                                    crate::touch::TouchGesture::Scrub { pct } => {
                                        let mut st = app_state.lock().unwrap();
                                        let allow_live_seek = last_touch_scrub_seek.elapsed()
                                            >= Duration::from_millis(80);
                                        if st.apply_touch_scrub(pct, allow_live_seek) {
                                            last_touch_scrub_seek = Instant::now();
                                        }
                                        window.request_redraw();
                                    }
                                    crate::touch::TouchGesture::ScrubEnd => {
                                        let mut st = app_state.lock().unwrap();
                                        st.finish_touch_scrub();
                                        window.request_redraw();
                                    }
                                    crate::touch::TouchGesture::ScrubCancel => {
                                        let mut st = app_state.lock().unwrap();
                                        st.cancel_touch_scrub();
                                        window.request_redraw();
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    if let WindowEvent::HoveredFile(path) = event {
                        if let Some(live_pos) = query_window_cursor_pos(window, force_unit_scale) {
                            last_cursor_pos = Some(live_pos);
                        }
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = Some(path.to_string_lossy().into_owned());
                        state.drag_cursor_pos = last_cursor_pos.map(|p| [p.x, p.y]);
                        if engine.track_info_rect.is_some() {
                            state.track_info_rect = engine.track_info_rect;
                        }
                        window.request_redraw();
                    } else if let WindowEvent::HoveredFileCancelled = event {
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = None;
                        state.drag_cursor_pos = None;
                        window.request_redraw();
                    }

                    if let WindowEvent::ModifiersChanged(m) = &event {
                        modifiers = m.state();
                    }

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
                                if state.file_loaded && state.duration_seconds > 0.0 {
                                    let duration = state.duration_seconds;
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
                                    clear_spectrum_history_in_place(&mut state);

                                    state.record_seek_osd(delta);
                                }
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
                                            let sel = state.selected_audio_track;
                                            if state.active_audio_tracks.is_empty() {
                                                state.active_audio_tracks.push(sel);
                                            }
                                            let num_tracks = state.audio_tracks.len().max(sel + 1);
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
                                            let sel = state.selected_audio_track;
                                            if state.active_audio_tracks.is_empty() {
                                                state.active_audio_tracks.push(sel);
                                            }
                                            let num_tracks = state.audio_tracks.len().max(sel + 1);
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
                                            let sel = state.selected_audio_track;
                                            if state.active_audio_tracks.is_empty() {
                                                state.active_audio_tracks.push(sel);
                                            }
                                            let num_tracks = state.audio_tracks.len().max(sel + 1);
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
                                                (state.panel_split_ratio - 0.05).clamp(
                                                    MIN_PANEL_SPLIT_RATIO,
                                                    MAX_PANEL_SPLIT_RATIO,
                                                );
                                            if state.file_loaded {
                                                state.show_hud = true;
                                                if state.video_mode == 3 {
                                                    state.video_mode = 0;
                                                }
                                            }
                                        }
                                        WinitKeyCode::BracketRight => {
                                            let mut state = app_state.lock().unwrap();
                                            state.panel_split_ratio =
                                                (state.panel_split_ratio + 0.05).clamp(
                                                    MIN_PANEL_SPLIT_RATIO,
                                                    MAX_PANEL_SPLIT_RATIO,
                                                );
                                            if state.file_loaded {
                                                state.show_hud = true;
                                                if state.video_mode == 3 {
                                                    state.video_mode = 0;
                                                }
                                            }
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
                                                    force_unit_scale,
                                                );
                                                keep_awake = None;
                                            }
                                        }
                                        WinitKeyCode::KeyU | WinitKeyCode::KeyE => {
                                            let mut state = app_state.lock().unwrap();
                                            state.toggle_surround_upmix();
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
                                                    state.vis_user_overridden[idx] = true;
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
                                                state.osd_text = Some("Playing".to_string());
                                                state.osd_timer = 2.0;
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
                                                state.osd_text = Some("Playing".to_string());
                                                state.osd_timer = 2.0;
                                            } else if state.file_loaded {
                                                state.is_paused = !state.is_paused;
                                                state.osd_text = Some(if state.is_paused {
                                                    "Paused".to_string()
                                                } else {
                                                    "Playing".to_string()
                                                });
                                                state.osd_timer = 2.0;
                                            }
                                            window.request_redraw();
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
                                                let cursor = state.vis_picker_cursor;
                                                state.vis_user_overridden[cursor] = true;
                                                state.current_visualizer_idx = cursor;
                                                state.visualizer_mode =
                                                    crate::state::VISUALIZERS[cursor].id;
                                                state.osd_text = Some(
                                                    crate::state::VISUALIZERS[cursor]
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
                                                            let vol = state
                                                                .audio_track_volumes
                                                                .get(track_idx)
                                                                .copied()
                                                                .unwrap_or(1.0);
                                                            state.audio_track_request =
                                                                Some(track_idx);
                                                            state.audio_mix_request =
                                                                Some(vec![(track_idx, vol)]);
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
                                    app_state.lock().unwrap().sync_rt_control();
                                }
                            }
                        }
                    }

                    if let WindowEvent::DroppedFile(path) = &event {
                        if let Some(live_pos) = query_window_cursor_pos(window, force_unit_scale) {
                            last_cursor_pos = Some(live_pos);
                        }
                        if drop_cursor_pos.is_none() {
                            drop_cursor_pos = last_cursor_pos.map(|p| [p.x, p.y]);
                        }
                        let path_str = crate::lyrics::normalize_path_str(&path.to_string_lossy());
                        if !path_str.is_empty() {
                            pending_dropped_files.push(path_str);
                        }
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = None;
                        state.drag_cursor_pos = None;
                        if engine.track_info_rect.is_some() {
                            state.track_info_rect = engine.track_info_rect;
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
                        WindowEvent::Moved(_) => {
                            if let Some(mhz) = window
                                .current_monitor()
                                .and_then(|m| m.refresh_rate_millihertz())
                            {
                                let hz = (mhz as f32 / 1000.0).clamp(30.0, 500.0);
                                app_state.lock().unwrap().target_fps = hz.round() as u32;
                            }
                        }
                        WindowEvent::ScaleFactorChanged { .. } => {
                            if force_unit_scale {
                                egui_ctx
                                    .set_zoom_factor(1.0 / (window.scale_factor() as f32).max(0.1));
                            }
                            engine.resize(window.inner_size());
                        }
                        WindowEvent::Resized(physical_size) => {
                            engine.resize(*physical_size);
                        }
                        WindowEvent::RedrawRequested => {
                            if is_first_frame {
                                is_first_frame = false;
                                let mut state = app_state.lock().unwrap();
                                state.is_paused = false;
                                state.sync_rt_control();
                            }

                            if !pending_dropped_files.is_empty() {
                                let raw_files = std::mem::take(&mut pending_dropped_files);
                                let expanded = crate::playlist::expand_input_paths(&raw_files);
                                let c_pos = drop_cursor_pos
                                    .take()
                                    .or_else(|| last_cursor_pos.map(|p| [p.x, p.y]));
                                let mut state = app_state.lock().unwrap();
                                if engine.track_info_rect.is_some() {
                                    state.track_info_rect = engine.track_info_rect;
                                }
                                state.handle_dropped_files(expanded, c_pos);
                            }

                            let (load_path, resume_pos_on_reload) = {
                                let mut state = app_state.lock().unwrap();
                                state.sync_rt_control();
                                let mut resume_pos_on_reload: Option<f64> = None;
                                let active_mic = is_mic_launch || state.is_mic_input;

                                let resolve_reload_path =
                                    |st: &AppState, mic_mode: bool| -> String {
                                        if mic_mode {
                                            if let Some(ref dev) = st.selected_audio_device {
                                                crate::state::mic_uri_for_device(dev)
                                            } else {
                                                "".to_string()
                                            }
                                        } else if st.playlist_index < st.playlist.len() {
                                            st.playlist[st.playlist_index].clone()
                                        } else {
                                            st.song_title.clone()
                                        }
                                    };

                                if state.audio_device_lost {
                                    state.audio_device_lost = false;
                                    let reload_path = resolve_reload_path(&state, active_mic);

                                    if !reload_path.is_empty() || active_mic {
                                        state.load_request = Some(reload_path);
                                        if !cli_upmix_override {
                                            state.surround_upmix.user_overridden = false;
                                        }
                                        resume_pos_on_reload = Some(state.current_seconds);
                                        state.osd_text =
                                            Some("Audio Device Reconnected".to_string());
                                        state.osd_timer = 3.0;
                                    }
                                }

                                if let Some(osd_msg) = state.audio_topology_reload_request.take() {
                                    let reload_path = resolve_reload_path(&state, active_mic);

                                    if (!reload_path.is_empty() && state.file_loaded) || active_mic
                                    {
                                        state.load_request = Some(reload_path);
                                        if !cli_upmix_override {
                                            state.surround_upmix.user_overridden = false;
                                        }
                                        resume_pos_on_reload = Some(state.current_seconds);
                                        state.osd_text = Some(osd_msg);
                                        state.osd_timer = 3.0;
                                    }
                                }

                                let device_change = state.audio_device_change_request.take();
                                if let Some(new_device) = device_change {
                                    let reload_path = resolve_reload_path(&state, active_mic);

                                    if !reload_path.is_empty() || active_mic {
                                        if active_mic && !state.playlist.is_empty() {
                                            std::sync::Arc::make_mut(&mut state.playlist)[0] =
                                                reload_path.clone();
                                        }
                                        state.load_request = Some(reload_path);
                                        state.osd_text = Some(if active_mic {
                                            format!("Audio Input: {}", new_device)
                                        } else {
                                            format!("Audio Output: {}", new_device)
                                        });
                                        if !cli_upmix_override {
                                            state.surround_upmix.user_overridden = false;
                                        }
                                        resume_pos_on_reload = Some(state.current_seconds);
                                        state.osd_timer = 3.0;
                                    }
                                }

                                if let Some(pt_enabled) = state.passthrough_change_request.take() {
                                    let reload_path = resolve_reload_path(&state, active_mic);

                                    if (!reload_path.is_empty() && state.file_loaded) || active_mic
                                    {
                                        state.load_request = Some(reload_path);
                                        resume_pos_on_reload = Some(state.current_seconds);
                                        reconcile_passthrough_change_osd(&mut state, pt_enabled);
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

                                if resume_pos_on_reload.is_none()
                                    && state.load_request.is_some()
                                    && !cli_upmix_override
                                {
                                    state.surround_upmix.user_overridden = false;
                                }
                                state.sync_rt_control();
                                (state.load_request.take(), resume_pos_on_reload)
                            };

                            if let Some(path) = load_path {
                                let is_mic = crate::state::is_mic_uri(&path)
                                    || ((is_mic_launch || app_state.lock().unwrap().is_mic_input)
                                        && path.is_empty());
                                {
                                    let mut state = app_state.lock().unwrap();
                                    let preferred_dev = crate::state::device_from_mic_uri(&path)
                                        .map(|s| s.to_string());
                                    state.set_mic_input_mode(is_mic, preferred_dev);
                                }
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
                                        state.stats.mpegh_bitstream_active = false;
                                        state.stats.mpegh_carrier_info.clear();
                                        state.stats.audio_buffer_fill_pct = 0.0;
                                        state.stats.video_buffer_fill_pct = 0.0;
                                        state.stats.video_decode_us = 0.0;
                                        state.stats.video_hw_decoder.clear();
                                        state.stats.video_fps = 0.0;
                                        state.stats.video_bitrate_kbps = None;
                                        state.stats.audio_bitrate_kbps = None;
                                        state.track_ended = false;
                                        let _ = state.rt_control.take_track_ended();
                                        state.sync_rt_control();
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
                                        let _ = state.rt_control.take_track_ended();
                                        if let Some(pos) = resume_pos_on_reload {
                                            if pos > 0.1 {
                                                state.current_seconds = pos;
                                                state.seek_request = Some(pos);
                                                state.seek_epoch += 1;
                                            }
                                            let preserve_device_osd =
                                                state.osd_text.as_ref().is_some_and(|s| {
                                                    s.contains("Audio Output:")
                                                        || s.contains("Audio Input:")
                                                        || s.starts_with("Audio Device")
                                                });
                                            if state.surround_upmix.enabled {
                                                if state.hardware_channels > 2 {
                                                    if !preserve_device_osd {
                                                        let badge =
                                                            crate::state::SurroundUpmixConfig::layout_badge(
                                                                state.hardware_channels,
                                                            );
                                                        if state.stats.mpegh_bitstream_active
                                                            && !state
                                                                .stats
                                                                .mpegh_carrier_info
                                                                .is_empty()
                                                        {
                                                            state.osd_text = Some(format!(
                                                                "🔊 11.1 ({badge}): {}",
                                                                state.stats.mpegh_carrier_info
                                                            ));
                                                        } else if state
                                                            .surround_upmix
                                                            .mpegh_bitstream
                                                            && state
                                                                .surround_upmix
                                                                .mpegh_user_overridden
                                                            && state.hardware_channels < 12
                                                        {
                                                            state.osd_text = Some(format!(
                                                                "🔊 11.1 requires 'Dolby Atmos for Home Theater' in Windows Sound Settings — Playing {badge}"
                                                            ));
                                                        } else {
                                                            state.osd_text = Some(format!(
                                                                "🔊 Auto Surround Upmix: ON ({badge})"
                                                            ));
                                                        }
                                                        state.show_hud = true;
                                                    }
                                                    if state.video_mode == 3 {
                                                        state.video_mode = 0;
                                                    }
                                                } else if !preserve_device_osd {
                                                    state.osd_text = Some(
                                                        "🔊 Auto Surround Upmix: Armed (Stereo/Mono Output — No Change)".to_string(),
                                                    );
                                                }
                                                state.osd_timer = 3.0;
                                            }
                                        }
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
                            desktop_frame_count += 1;
                            if desktop_frame_count == 1
                                && let Some(mhz) = window
                                    .current_monitor()
                                    .or_else(|| window.primary_monitor())
                                    .and_then(|m| m.refresh_rate_millihertz())
                            {
                                let hz = (mhz as f32 / 1000.0).clamp(30.0, 500.0);
                                frame_dt_ring = [1.0 / hz; 60];
                            } else if desktop_frame_count > 2 && raw_dt > 0.0 && raw_dt < 0.25 {
                                frame_dt_ring[frame_dt_idx] = raw_dt;
                                frame_dt_idx = (frame_dt_idx + 1) % frame_dt_ring.len();
                            }
                            let avg_dt =
                                frame_dt_ring.iter().sum::<f32>() / (frame_dt_ring.len() as f32);
                            let fps = if avg_dt > 0.0 { 1.0 / avg_dt } else { 60.0 };
                            let time_scale = (avg_dt * 60.0).clamp(0.15, 3.0); // Decay logic built for 60fps

                            let phase_timer = Instant::now();
                            {
                                let mut state = app_state.lock().unwrap();
                                state.current_fps = fps;
                                if bench.is_none() {
                                    let target_fps = state.target_fps.max(1) as f32;
                                    state.update_visualizer_perf_guard(avg_dt, target_fps);
                                }
                                state.visual_width = engine.config.width / 2;

                                if state.file_loaded {
                                    if state.is_paused || state.track_ended {
                                        state.raw_channel_vus.fill(0.0);
                                        state.raw_spectrum_data.fill(0.0);
                                        if state.gpu_spectrum_data.iter().any(|&v| v != 0.0) {
                                            state.gpu_spectrum_data.fill(0.0);
                                            state.waveform_history_push_count =
                                                state.waveform_history_push_count.wrapping_add(1);
                                        }
                                    }
                                    let dt_sec = avg_dt.clamp(0.002, 0.05);
                                    let vu_decay = 10.0_f32.powf(-20.0 * dt_sec / 20.0);
                                    let peak_decay = 10.0_f32.powf(-8.6 * dt_sec / 20.0);
                                    if state.channel_vus.len() != state.raw_channel_vus.len() {
                                        state.channel_vus = vec![0.0; state.raw_channel_vus.len()];
                                    }
                                    for i in 0..state.raw_channel_vus.len() {
                                        if state.raw_channel_vus[i] > state.channel_vus[i] {
                                            state.channel_vus[i] = state.raw_channel_vus[i];
                                        } else {
                                            let decayed = state.channel_vus[i] * vu_decay;
                                            state.channel_vus[i] =
                                                if decayed < 1e-4 { 0.0 } else { decayed }
                                                    .max(state.raw_channel_vus[i]);
                                        }
                                    }

                                    if state.peak_vus.len() != state.channel_vus.len() {
                                        state.peak_vus = vec![0.0; state.channel_vus.len()];
                                    }
                                    for i in 0..state.channel_vus.len() {
                                        let decayed_peak = state.peak_vus[i] * peak_decay;
                                        state.peak_vus[i] = if decayed_peak < 1e-4 {
                                            0.0
                                        } else {
                                            decayed_peak
                                        };
                                        if state.channel_vus[i] > state.peak_vus[i] {
                                            state.peak_vus[i] = state.channel_vus[i];
                                        }
                                    }

                                    if state.spectrum_data.len() != state.raw_spectrum_data.len() {
                                        state.spectrum_data =
                                            vec![0.0; state.raw_spectrum_data.len()];
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
                                    // Scroll spectrum history (reuse front allocation)
                                    let mut recycled = if state.spectrum_history.len() > 120 {
                                        state.spectrum_history.pop_front().unwrap_or_default()
                                    } else {
                                        Vec::with_capacity(state.spectrum_data.len())
                                    };
                                    recycled.clear();
                                    recycled.extend_from_slice(&state.spectrum_data);
                                    state.spectrum_history.push_back(recycled);
                                }

                                // Gamepad analog panel scaling (Left Stick Y or Right Stick Y) and trigger scrubbing
                                let mut max_left_y = std::mem::take(&mut frame_left_stick_y);
                                let mut max_right_y = std::mem::take(&mut frame_right_stick_y);
                                let mut max_lt = std::mem::take(&mut frame_left_trigger);
                                let mut max_rt = std::mem::take(&mut frame_right_trigger);
                                for (_id, gamepad) in gilrs.gamepads() {
                                    let left_y = gamepad.value(gilrs::Axis::LeftStickY);
                                    if left_y.abs() > max_left_y.abs() {
                                        max_left_y = left_y;
                                    }
                                    let right_y = gamepad.value(gilrs::Axis::RightStickY);
                                    if right_y.abs() > max_right_y.abs() {
                                        max_right_y = right_y;
                                    }

                                    let lt = gamepad
                                        .button_data(gilrs::Button::LeftTrigger2)
                                        .map(|d| d.value())
                                        .unwrap_or_else(|| {
                                            if gamepad.is_pressed(gilrs::Button::LeftTrigger2) {
                                                1.0
                                            } else {
                                                0.0
                                            }
                                        });
                                    if lt > max_lt {
                                        max_lt = lt;
                                    }
                                    let rt = gamepad
                                        .button_data(gilrs::Button::RightTrigger2)
                                        .map(|d| d.value())
                                        .unwrap_or_else(|| {
                                            if gamepad.is_pressed(gilrs::Button::RightTrigger2) {
                                                1.0
                                            } else {
                                                0.0
                                            }
                                        });
                                    if rt > max_rt {
                                        max_rt = rt;
                                    }
                                }
                                apply_gamepad_trigger_scrub(&mut state, max_lt, max_rt, dt);
                                apply_gamepad_stick_panel_resize(
                                    &mut state,
                                    max_left_y,
                                    max_right_y,
                                    dt,
                                );

                                state.tick_osd_timer(dt);

                                // Clamp panel split ratio to ensure both top and bottom panels have at least MIN_PANEL_HEIGHT_PX height
                                let total_h = engine.config.height as f32
                                    / effective_window_scale(window, force_unit_scale) as f32;
                                state.panel_split_ratio =
                                    clamp_panel_split_ratio(state.panel_split_ratio, total_h);

                                state.sync_rt_control();
                                engine.update(&state, dt);
                            }
                            let phase_lock_update_us = phase_timer.elapsed().as_micros() as f32;

                            // Create a lightweight snapshot for render (skips ~1.3 MB of audio data)
                            // and drain gamepad events in the same lock acquisition
                            let snapshot_timer = Instant::now();
                            let (state_copy, gamepad_events) = {
                                let is_hovering_file =
                                    app_state.lock().unwrap().hovered_file.is_some();
                                if is_hovering_file
                                    && let Some(live_pos) =
                                        query_window_cursor_pos(window, force_unit_scale)
                                {
                                    last_cursor_pos = Some(live_pos);
                                }
                                let mut state = app_state.lock().unwrap();
                                if state.hovered_file.is_some() {
                                    state.drag_cursor_pos = last_cursor_pos.map(|p| [p.x, p.y]);
                                }
                                state.track_info_rect = engine.track_info_rect;
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
                                state.track_info_rect = engine.track_info_rect;

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
                                        state.stats.gpu_compute_us =
                                            state.stats.gpu_compute_us * 0.9 + ft * 0.1;
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
                                            "BENCHMARK_RESULT_GPU_COMPUTE_US: {:.2}",
                                            state.stats.gpu_compute_us
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
                                        state.is_touch_scrubbing = false;
                                        if let Some(target) = state.scrub_target_seconds.take() {
                                            state.seek_request = Some(target);
                                            state.current_seconds = target;
                                            state.track_ended = false;
                                            clear_spectrum_history_in_place(&mut state);
                                        }
                                    }
                                    EngineAction::Seek(pct) => {
                                        let target = (state.duration_seconds * pct as f64)
                                            .clamp(0.0, state.duration_seconds);
                                        state.seek_request = Some(target);
                                        state.current_seconds = target;
                                        state.track_ended = false;
                                        state.is_touch_scrubbing = false;
                                        state.scrub_target_seconds = None;
                                        clear_spectrum_history_in_place(&mut state);
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
                                        let expanded = crate::playlist::expand_input_paths(&paths);
                                        if append
                                            && !state.playlist.is_empty()
                                            && state.file_loaded
                                            && !state.is_mic_input
                                        {
                                            if !expanded.is_empty() {
                                                let count = expanded.len();
                                                let first_name = std::path::Path::new(&expanded[0])
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
                                                std::sync::Arc::make_mut(&mut state.playlist)
                                                    .extend(expanded);
                                            }
                                        } else if !expanded.is_empty() {
                                            state.set_mic_input_mode(false, None);
                                            state.playlist = std::sync::Arc::new(expanded);
                                            state.playlist_index = 0;
                                            state.load_request = Some(state.playlist[0].clone());
                                            state.file_loaded = true;
                                            state.is_paused = false;
                                            state.track_ended = false;
                                        }
                                        state.is_file_picker_open = false;
                                    }
                                    EngineAction::SetAppendToPlaylist(val) => {
                                        state.append_to_playlist = val;
                                    }
                                    EngineAction::SetForceStereo(val) => {
                                        state.set_force_stereo_downmix(val);
                                    }
                                    EngineAction::ToggleSurroundUpmix(val) => {
                                        state.set_surround_upmix(val);
                                    }
                                    EngineAction::ToggleMpeghBitstream(val) => {
                                        state.set_mpegh_bitstream(val);
                                    }
                                    EngineAction::SetSurroundUpmix(cfg) => {
                                        state.surround_upmix = cfg;
                                    }
                                    EngineAction::SetShowSurroundEq(val) => {
                                        state.set_show_surround_eq(val);
                                    }
                                    EngineAction::SetPassthrough(val) => {
                                        if cfg!(any(target_os = "windows", target_os = "linux")) {
                                            state.passthrough_enabled = val;
                                            if val {
                                                state.surround_upmix.enabled = false;
                                                state.force_stereo_downmix = false;
                                                state.show_surround_eq = false;
                                            }
                                            state.osd_text = Some(if val {
                                                "Bitstream Passthrough: ON (Exclusive)".to_string()
                                            } else {
                                                "Bitstream Passthrough: OFF (Shared Mode)"
                                                    .to_string()
                                            });
                                            state.osd_timer = 3.0;
                                            state.passthrough_change_request = Some(val);
                                        } else {
                                            state.passthrough_enabled = false;
                                            state.osd_text = Some(
                                                "Bitstream Passthrough requires Linux or Windows"
                                                    .to_string(),
                                            );
                                            state.osd_timer = 2.5;
                                        }
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
                                        state.vis_user_overridden[idx] = true;
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
                                            state.vis_user_overridden[idx] = true;
                                        }
                                    }
                                    EngineAction::VisPickerSetCursor(idx) => {
                                        state.vis_picker_cursor = idx;
                                    }
                                    EngineAction::VisPickerEnableAll => {
                                        for i in 0..state.vis_enabled.len() {
                                            state.vis_enabled[i] = true;
                                            state.vis_user_overridden[i] = true;
                                        }
                                    }
                                    EngineAction::VisPickerEnableNone => {
                                        for i in 0..state.vis_enabled.len() {
                                            // Index 0 (Frequency Spectrum) is always enabled
                                            state.vis_enabled[i] = i == 0;
                                        }
                                    }
                                    EngineAction::OpenUrlDialog => {
                                        state.available_input_devices =
                                            crate::audio::get_available_audio_devices(true);
                                        if state.is_mic_input {
                                            state.available_audio_devices =
                                                state.available_input_devices.clone();
                                        }
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
                                    EngineAction::LoadInputDevice(device_name) => {
                                        let mic_uri =
                                            crate::state::mic_uri_for_device(&device_name);
                                        state.is_url_dialog_open = false;
                                        window.set_ime_allowed(false);
                                        state.set_mic_input_mode(true, Some(device_name.clone()));
                                        state.playlist = std::sync::Arc::new(vec![mic_uri.clone()]);
                                        state.playlist_index = 0;
                                        state.load_request = Some(mic_uri);
                                        state.file_loaded = true;
                                        state.is_paused = false;
                                        state.track_ended = false;
                                        state.osd_text =
                                            Some(format!("🔊 Audio Input: {}", device_name));
                                        state.osd_timer = 3.0;
                                    }
                                    EngineAction::LoadUrl(url) => {
                                        let mut final_url = url.clone();
                                        if crate::state::is_mic_uri(&final_url) {
                                            let dev_name =
                                                crate::state::device_from_mic_uri(&final_url)
                                                    .unwrap_or(
                                                        crate::audio::SYSTEM_DEFAULT_AUDIO_DEVICE,
                                                    )
                                                    .to_string();
                                            state.is_url_dialog_open = false;
                                            window.set_ime_allowed(false);
                                            state.url_input_text.clear();
                                            state.set_mic_input_mode(true, Some(dev_name.clone()));
                                            let mic_uri =
                                                crate::state::mic_uri_for_device(&dev_name);
                                            state.playlist =
                                                std::sync::Arc::new(vec![mic_uri.clone()]);
                                            state.playlist_index = 0;
                                            state.load_request = Some(mic_uri);
                                            state.file_loaded = true;
                                            state.is_paused = false;
                                            state.track_ended = false;
                                            state.osd_text =
                                                Some(format!("🔊 Audio Input: {}", dev_name));
                                            state.osd_timer = 3.0;
                                        } else {
                                            if !final_url.starts_with("http://")
                                                && !final_url.starts_with("https://")
                                            {
                                                final_url = format!("https://{}", final_url);
                                            }
                                            state.set_mic_input_mode(false, None);
                                            state.is_url_dialog_open = false;
                                            window.set_ime_allowed(false);
                                            state.url_input_text.clear();
                                            state.playlist =
                                                std::sync::Arc::new(vec![final_url.clone()]);
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
                                    }
                                    EngineAction::None => {}
                                }

                                // File picker state
                                let fd_state = file_dialog.state();
                                state.is_file_picker_open =
                                    *fd_state == egui_file_dialog::DialogState::Open;
                                if *fd_state == egui_file_dialog::DialogState::Cancelled {
                                    file_dialog = egui_file_dialog::FileDialog::new()
                                        .default_size([920.0, 560.0])
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
                                        if append
                                            && !state.playlist.is_empty()
                                            && !state.is_mic_input
                                        {
                                            std::sync::Arc::make_mut(&mut state.playlist)
                                                .extend(expanded);
                                        } else if !expanded.is_empty() {
                                            state.set_mic_input_mode(false, None);
                                            state.playlist = std::sync::Arc::new(expanded);
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
                                            if !state.file_loaded
                                                || state.playlist.is_empty()
                                                || state.is_mic_input
                                            {
                                                state.set_mic_input_mode(false, None);
                                                state.playlist =
                                                    std::sync::Arc::new(expanded.clone());
                                                state.playlist_index = 0;
                                                state.load_request =
                                                    Some(state.playlist[0].clone());
                                                state.file_loaded = true;
                                            } else {
                                                let first_new_idx = state.playlist.len();
                                                std::sync::Arc::make_mut(&mut state.playlist)
                                                    .extend(expanded.clone());
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
                                state.sync_rt_control();
                            }

                            if trigger_picker {
                                egui_ctx.memory_mut(|m| {
                                    if let Some(id) = m.focused() {
                                        m.surrender_focus(id);
                                    }
                                });
                                file_dialog.pick_multiple();
                                let mut state = app_state.lock().unwrap();
                                state.egui_gamepad_events.push(egui::Event::Key {
                                    key: egui::Key::ArrowDown,
                                    physical_key: None,
                                    pressed: true,
                                    repeat: false,
                                    modifiers: egui::Modifiers::NONE,
                                });
                                state.egui_gamepad_events.push(egui::Event::Key {
                                    key: egui::Key::ArrowDown,
                                    physical_key: None,
                                    pressed: false,
                                    repeat: false,
                                    modifiers: egui::Modifiers::NONE,
                                });
                            }

                            // Hardware VSync (PresentMode::Fifo) paces frames natively to VBlank.
                            // Only apply software sleep throttling when RUSTTRACKER_FPS_LIMIT is
                            // explicitly requested (and not running --uncapped or --bench), as
                            // unconditional post-present CPU sleeping fights swapchain VSync and
                            // Windows DWM composition in windowed mode.
                            let fps_limit = if !is_uncapped && bench.is_none() {
                                std::env::var("RUSTTRACKER_FPS_LIMIT")
                                    .ok()
                                    .and_then(|s| s.trim().parse::<f32>().ok())
                                    .filter(|&fps| fps.is_finite() && fps > 0.0)
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
                    if !pending_dropped_files.is_empty() {
                        let raw_files = std::mem::take(&mut pending_dropped_files);
                        let expanded = crate::playlist::expand_input_paths(&raw_files);
                        let c_pos = drop_cursor_pos
                            .take()
                            .or_else(|| last_cursor_pos.map(|p| [p.x, p.y]));
                        let mut state = app_state.lock().unwrap();
                        if engine.track_info_rect.is_some() {
                            state.track_info_rect = engine.track_info_rect;
                        }
                        state.handle_dropped_files(expanded, c_pos);
                        window.request_redraw();
                    }

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
                                std::sync::Arc::make_mut(&mut state.playlist).extend(expanded);
                            } else if !expanded.is_empty() {
                                state.playlist = std::sync::Arc::new(expanded);
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
                                    state.playlist = std::sync::Arc::new(expanded.clone());
                                    state.playlist_index = 0;
                                    state.load_request = Some(state.playlist[0].clone());
                                    state.file_loaded = true;
                                } else {
                                    let first_new_idx = state.playlist.len();
                                    std::sync::Arc::make_mut(&mut state.playlist)
                                        .extend(expanded.clone());
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
                        match g_event {
                            gilrs::EventType::AxisChanged(axis, val, _) => match axis {
                                gilrs::Axis::LeftStickY if val.abs() > frame_left_stick_y.abs() => {
                                    frame_left_stick_y = val;
                                }
                                gilrs::Axis::RightStickY
                                    if val.abs() > frame_right_stick_y.abs() =>
                                {
                                    frame_right_stick_y = val;
                                }
                                _ => {}
                            },
                            gilrs::EventType::ButtonChanged(btn, val, _) => match btn {
                                gilrs::Button::LeftTrigger2 if val > frame_left_trigger => {
                                    frame_left_trigger = val;
                                }
                                gilrs::Button::RightTrigger2 if val > frame_right_trigger => {
                                    frame_right_trigger = val;
                                }
                                _ => {}
                            },
                            _ => {}
                        }
                        let is_pressed = matches!(g_event, gilrs::EventType::ButtonPressed(_, _));
                        let is_repeated = matches!(g_event, gilrs::EventType::ButtonRepeated(_, _));
                        if is_pressed || is_repeated {
                            let button = match g_event {
                                gilrs::EventType::ButtonPressed(b, _) => b,
                                gilrs::EventType::ButtonRepeated(b, _) => b,
                                _ => unreachable!(),
                            };

                            if is_repeated {
                                if !is_dialog_open
                                    && matches!(
                                        button,
                                        gilrs::Button::DPadLeft | gilrs::Button::DPadRight
                                    )
                                {
                                    let mut state = app_state.lock().unwrap();
                                    let _ = apply_gamepad_playback_button(&mut state, button);
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
                                            force_unit_scale,
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
                                        if is_url_open {
                                            push_key(egui::Key::ArrowLeft);
                                        } else {
                                            egui_ctx.memory_mut(|m| {
                                                if let Some(id) = m.focused() {
                                                    m.surrender_focus(id);
                                                }
                                            });
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::ArrowUp,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers: egui::Modifiers::ALT,
                                            });
                                            state.egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::ArrowUp,
                                                physical_key: None,
                                                pressed: false,
                                                repeat: false,
                                                modifiers: egui::Modifiers::ALT,
                                            });
                                        }
                                        continue;
                                    }
                                    gilrs::Button::DPadRight => {
                                        if is_url_open {
                                            push_key(egui::Key::ArrowRight);
                                        } else {
                                            egui_ctx.memory_mut(|m| {
                                                if let Some(id) = m.focused() {
                                                    m.surrender_focus(id);
                                                }
                                            });
                                            push_key(egui::Key::Enter);
                                        }
                                        continue;
                                    }
                                    gilrs::Button::South => {
                                        if !is_url_open {
                                            egui_ctx.memory_mut(|m| {
                                                if let Some(id) = m.focused() {
                                                    m.surrender_focus(id);
                                                }
                                            });
                                        }
                                        push_key(egui::Key::Enter);
                                        continue;
                                    }
                                    gilrs::Button::East => {
                                        if !is_url_open {
                                            egui_ctx.memory_mut(|m| {
                                                if let Some(id) = m.focused() {
                                                    m.surrender_focus(id);
                                                }
                                            });
                                        }
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
                                    !state.file_loaded && !state.show_vis_picker
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
                                        gilrs::Button::DPadLeft | gilrs::Button::DPadUp => {
                                            push_tab(true);
                                            continue;
                                        }
                                        gilrs::Button::DPadRight | gilrs::Button::DPadDown => {
                                            push_tab(false);
                                            continue;
                                        }
                                        gilrs::Button::South => {
                                            if egui_ctx.memory(|m| m.focused().is_none()) {
                                                state.open_file_request = true;
                                            } else {
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
                                            }
                                            continue;
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            {
                                let mut state = app_state.lock().unwrap();
                                apply_gamepad_playback_button(&mut state, button);
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

                    if window.is_minimized() == Some(true) {
                        std::thread::sleep(Duration::from_millis(16));
                    } else {
                        window.request_redraw();
                    }
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

fn adjust_tui_volume(state: &mut AppState, delta: f32) {
    let sel = state.selected_audio_track;
    if state.active_audio_tracks.is_empty() {
        state.active_audio_tracks.push(sel);
    }
    let num_tracks = state.audio_tracks.len().max(sel + 1);
    if state.audio_track_volumes.len() != num_tracks {
        state.audio_track_volumes = vec![1.0; num_tracks];
    }
    for vol in &mut state.audio_track_volumes {
        *vol = (*vol + delta).clamp(0.0, 1.0);
    }
    let current_vol = state.audio_track_volumes.first().copied().unwrap_or(1.0);
    let mix: Vec<(usize, f32)> = state
        .active_audio_tracks
        .iter()
        .map(|&idx| {
            (
                idx,
                state.audio_track_volumes.get(idx).copied().unwrap_or(1.0),
            )
        })
        .collect();
    state.audio_mix_request = Some(mix);
    state.osd_text = Some(format!("Volume: {}%", (current_vol * 100.0).round() as u32));
    state.osd_timer = 2.0;
}

pub(crate) fn handle_tui_key_event(state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyEventKind, KeyModifiers};

    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
        return false;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            key.code,
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Char('q') | KeyCode::Char('Q')
        )
    {
        return true;
    }

    if state.show_tui_device_picker {
        match key.code {
            KeyCode::Esc
            | KeyCode::Char('d')
            | KeyCode::Char('D')
            | KeyCode::Char('q')
            | KeyCode::Char('Q') => {
                state.show_tui_device_picker = false;
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                let count = state.available_audio_devices.len();
                if count > 0 {
                    if state.tui_device_picker_cursor == 0 {
                        state.tui_device_picker_cursor = count - 1;
                    } else {
                        state.tui_device_picker_cursor -= 1;
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                let count = state.available_audio_devices.len();
                if count > 0 {
                    state.tui_device_picker_cursor = (state.tui_device_picker_cursor + 1) % count;
                }
            }
            KeyCode::Enter => {
                let cursor = state.tui_device_picker_cursor;
                if cursor < state.available_audio_devices.len() {
                    let dev = state.available_audio_devices[cursor].clone();
                    state.audio_device_change_request = Some(dev);
                }
                state.show_tui_device_picker = false;
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            return true;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            let refreshed = crate::audio::get_available_audio_devices(state.is_mic_input);
            if !refreshed.is_empty() {
                state.available_audio_devices = refreshed;
            }
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
        KeyCode::Char('v') | KeyCode::Char('V') | KeyCode::Tab => {
            state.cycle_tui_center_view();
        }
        KeyCode::Char('u') | KeyCode::Char('U') | KeyCode::Char('e') | KeyCode::Char('E') => {
            state.toggle_surround_upmix();
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            if state.effective_tui_center_view() == crate::state::TuiCenterView::SurroundEq {
                state.toggle_mpegh_bitstream();
            } else {
                let sel = state.selected_audio_track;
                if state.active_audio_tracks.is_empty() {
                    state.active_audio_tracks.push(sel);
                }
                let num_tracks = state.audio_tracks.len().max(sel + 1);
                if state.audio_track_volumes.len() != num_tracks {
                    state.audio_track_volumes = vec![1.0; num_tracks];
                }
                let is_currently_zero = state.audio_track_volumes.iter().all(|&v| v == 0.0);
                let new_vol = if is_currently_zero { 1.0 } else { 0.0 };
                state.audio_track_volumes.fill(new_vol);
                let mix: Vec<(usize, f32)> = state
                    .active_audio_tracks
                    .iter()
                    .map(|&idx| (idx, new_vol))
                    .collect();
                state.audio_mix_request = Some(mix);
                state.osd_text = Some(if new_vol == 0.0 {
                    "Muted".to_string()
                } else {
                    "Unmuted".to_string()
                });
                state.osd_timer = 2.0;
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if state.effective_tui_center_view() == crate::state::TuiCenterView::SurroundEq {
                state.surround_upmix.reset_tuning();
                state.osd_text = Some("Surround EQ Reset to Defaults".to_string());
                state.osd_timer = 2.0;
            }
        }
        KeyCode::Char('b') | KeyCode::Char('B') => {
            if cfg!(any(target_os = "windows", target_os = "linux")) {
                state.passthrough_enabled = !state.passthrough_enabled;
                if state.passthrough_enabled {
                    state.surround_upmix.enabled = false;
                    state.force_stereo_downmix = false;
                    state.show_surround_eq = false;
                    if state.tui_center_view == Some(crate::state::TuiCenterView::SurroundEq) {
                        state.tui_center_view = Some(crate::state::TuiCenterView::Heatmap);
                    }
                }
                state.osd_text = Some(if state.passthrough_enabled {
                    "Bitstream Passthrough: ON (Exclusive)".to_string()
                } else {
                    "Bitstream Passthrough: OFF (Shared Mode)".to_string()
                });
                state.osd_timer = 3.0;
                state.passthrough_change_request = Some(state.passthrough_enabled);
            } else {
                state.passthrough_enabled = false;
                state.osd_text =
                    Some("Bitstream Passthrough requires Linux or Windows".to_string());
                state.osd_timer = 2.5;
            }
        }
        KeyCode::Char('[') => {
            state.panel_split_ratio = (state.panel_split_ratio - 0.05)
                .clamp(MIN_PANEL_SPLIT_RATIO, MAX_PANEL_SPLIT_RATIO);
        }
        KeyCode::Char(']') => {
            state.panel_split_ratio = (state.panel_split_ratio + 0.05)
                .clamp(MIN_PANEL_SPLIT_RATIO, MAX_PANEL_SPLIT_RATIO);
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            adjust_tui_volume(state, 0.05);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            adjust_tui_volume(state, -0.05);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            if !state.playlist.is_empty() && state.playlist_index + 1 < state.playlist.len() {
                state.playlist_index += 1;
                state.load_request = Some(state.playlist[state.playlist_index].clone());
                state.is_paused = false;
                state.osd_text = Some("Next Track".to_string());
                state.osd_timer = 2.0;
            }
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            if !state.playlist.is_empty() && state.playlist_index > 0 {
                state.playlist_index -= 1;
                state.load_request = Some(state.playlist[state.playlist_index].clone());
                state.is_paused = false;
                state.osd_text = Some("Previous Track".to_string());
                state.osd_timer = 2.0;
            }
        }
        KeyCode::Char(' ') => {
            if state.track_ended
                || (state.current_seconds >= state.duration_seconds - 0.1
                    && state.duration_seconds > 0.0)
            {
                state.track_ended = false;
                state.seek_request = Some(0.0);
                for row in &mut state.spectrum_history {
                    row.fill(0.0);
                }
                state.is_paused = false;
                state.osd_text = Some("Playing".to_string());
                state.osd_timer = 2.0;
            } else if state.playlist_index >= state.playlist.len() && !state.playlist.is_empty() {
                if state.duration_seconds == 0.0 {
                    state.playlist_index = state.playlist.len() - 1;
                } else {
                    state.playlist_index = 0;
                }
                state.load_request = Some(state.playlist[state.playlist_index].clone());
                state.is_paused = false;
                state.osd_text = Some("Playing".to_string());
                state.osd_timer = 2.0;
            } else {
                state.is_paused = !state.is_paused;
                state.osd_text = Some(if state.is_paused {
                    "Paused".to_string()
                } else {
                    "Playing".to_string()
                });
                state.osd_timer = 2.0;
            }
        }
        KeyCode::Right | KeyCode::Left => {
            if state.duration_seconds > 0.0 {
                let duration = state.duration_seconds;
                let step = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    60.0f64.min(duration).max(5.0)
                } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                    30.0f64.min(duration).max(1.0)
                } else {
                    5.0f64.min(duration).max(0.5)
                };
                let delta = if key.code == KeyCode::Right {
                    step
                } else {
                    -step
                };
                let target = (state.current_seconds + delta).clamp(0.0, duration);
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                for row in &mut state.spectrum_history {
                    row.fill(0.0);
                }
                state.record_seek_osd(delta);
            }
        }
        KeyCode::Char(c @ '1'..='9') => {
            let track_num = (c as u8 - b'0') as usize;
            let num_tracks = state.audio_tracks.len();
            if num_tracks > 1 {
                if track_num <= num_tracks {
                    let track_idx = track_num - 1;
                    if state.active_audio_tracks != vec![track_idx]
                        || state.selected_audio_track != track_idx
                    {
                        state.selected_audio_track = track_idx;
                        state.active_audio_tracks = vec![track_idx];
                        if state.audio_track_volumes.len() != num_tracks {
                            state.audio_track_volumes = vec![1.0; num_tracks];
                        }
                        let vol = state
                            .audio_track_volumes
                            .get(track_idx)
                            .copied()
                            .unwrap_or(1.0);
                        state.audio_track_request = Some(track_idx);
                        state.audio_mix_request = Some(vec![(track_idx, vol)]);
                        state.osd_text = Some(format!("Audio Track {}/{}", track_num, num_tracks));
                        state.osd_timer = 2.0;
                    }
                } else if track_num == num_tracks + 1 {
                    if state.audio_track_volumes.len() != num_tracks {
                        state.audio_track_volumes = vec![1.0; num_tracks];
                    }
                    let mix: Vec<(usize, f32)> = (0..num_tracks)
                        .map(|idx| {
                            (
                                idx,
                                state.audio_track_volumes.get(idx).copied().unwrap_or(1.0),
                            )
                        })
                        .collect();
                    state.active_audio_tracks = (0..num_tracks).collect();
                    state.audio_mix_request = Some(mix);
                    state.osd_text = Some(format!("Multi-Track Mix (1-{})", num_tracks));
                    state.osd_timer = 2.0;
                }
            }
        }
        _ => {}
    }

    false
}

pub(crate) fn handle_tui_mouse_event(
    state: &mut AppState,
    mouse: crossterm::event::MouseEvent,
    term_area: ratatui::layout::Rect,
) {
    use crossterm::event::{MouseButton, MouseEventKind};

    let meta_len = crate::ui::build_track_info_lines(state, 30).len();
    let layout = crate::ui::compute_tui_layout(term_area, state, meta_len);
    let col = mouse.column;
    let row = mouse.row;
    let contains = |r: ratatui::layout::Rect, x: u16, y: u16| -> bool {
        x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
    };

    if let Some(modal) = layout.device_picker_rect {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let count = state.available_audio_devices.len();
                if count > 0 {
                    if state.tui_device_picker_cursor == 0 {
                        state.tui_device_picker_cursor = count - 1;
                    } else {
                        state.tui_device_picker_cursor -= 1;
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                let count = state.available_audio_devices.len();
                if count > 0 {
                    state.tui_device_picker_cursor = (state.tui_device_picker_cursor + 1) % count;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if contains(modal, col, row) {
                    let inner_h = modal.height.saturating_sub(2) as usize;
                    let count = state.available_audio_devices.len();
                    if row > modal.y && row + 1 < modal.bottom() && count > 0 && inner_h > 0 {
                        let cursor = state.tui_device_picker_cursor.min(count - 1);
                        let scroll_offset = if cursor >= inner_h {
                            cursor + 1 - inner_h
                        } else {
                            0
                        };
                        let clicked_idx = scroll_offset + (row - modal.y - 1) as usize;
                        if clicked_idx < count {
                            state.tui_device_picker_cursor = clicked_idx;
                            let dev = state.available_audio_devices[clicked_idx].clone();
                            state.audio_device_change_request = Some(dev);
                            state.show_tui_device_picker = false;
                        }
                    }
                } else {
                    state.show_tui_device_picker = false;
                }
            }
            _ => {}
        }
        return;
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if row == layout.timeline_rect.y
                && col >= layout.timeline_rect.x
                && col < layout.timeline_rect.right()
                && state.duration_seconds > 0.0
            {
                let frac = ((col - layout.timeline_rect.x) as f64
                    / (layout.timeline_rect.width.max(1) as f64))
                    .clamp(0.0, 1.0);
                let target = frac * state.duration_seconds;
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                for r in &mut state.spectrum_history {
                    r.fill(0.0);
                }
                let force_h = state.duration_seconds >= 3600.0;
                state.osd_text = Some(format!(
                    "Seek: {}",
                    crate::state::format_timeline_time(target, force_h)
                ));
                state.osd_timer = 1.5;
            } else if contains(layout.center_rect, col, row) {
                state.cycle_tui_center_view();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if row == layout.timeline_rect.y
                && col >= layout.timeline_rect.x
                && col < layout.timeline_rect.right()
                && state.duration_seconds > 0.0
            {
                let frac = ((col - layout.timeline_rect.x) as f64
                    / (layout.timeline_rect.width.max(1) as f64))
                    .clamp(0.0, 1.0);
                let target = frac * state.duration_seconds;
                state.seek_request = Some(target);
                state.current_seconds = target;
                state.track_ended = false;
                for r in &mut state.spectrum_history {
                    r.fill(0.0);
                }
                let force_h = state.duration_seconds >= 3600.0;
                state.osd_text = Some(format!(
                    "Seek: {}",
                    crate::state::format_timeline_time(target, force_h)
                ));
                state.osd_timer = 1.5;
            } else if row.abs_diff(layout.split_row_y) <= 2 && term_area.height > 12 {
                let content_h = term_area.height.saturating_sub(4).max(1) as f32;
                let rel_y = row.saturating_sub(1) as f32;
                state.panel_split_ratio =
                    (rel_y / content_h).clamp(MIN_PANEL_SPLIT_RATIO, MAX_PANEL_SPLIT_RATIO);
            }
        }
        MouseEventKind::ScrollUp => {
            adjust_tui_volume(state, 0.05);
        }
        MouseEventKind::ScrollDown => {
            adjust_tui_volume(state, -0.05);
        }
        _ => {}
    }
}

fn run_tui<B: Backend>(
    terminal: &mut Terminal<B>,
    app_state: Arc<Mutex<AppState>>,
    mut active_stream: Option<audio::PlaybackHandle>,
    is_mic_launch: bool,
    cli_upmix_override: bool,
) -> io::Result<()>
where
    std::io::Error: From<<B as Backend>::Error>,
{
    let tick_rate = Duration::from_millis(16); // ~60fps
    let mut last_tick = Instant::now();
    let mut last_frame = Instant::now();
    let mut is_first_frame = true;

    loop {
        if is_first_frame {
            is_first_frame = false;
            let mut state = app_state.lock().unwrap();
            state.is_paused = false;
            state.sync_rt_control();
        }

        let (load_path, resume_pos_on_reload) = {
            let mut state = app_state.lock().unwrap();
            state.sync_rt_control();
            let mut resume_pos_on_reload: Option<f64> = None;
            let active_mic = is_mic_launch || state.is_mic_input;

            // Drain any decoded video frames so the video decoder thread never blocks in TUI mode
            if let (Some(rx), Some(free_tx)) = (&state.video_frame_rx, &state.free_video_frame_tx) {
                while let Ok(frame) = rx.try_recv() {
                    let _ = free_tx.try_send(frame);
                }
            }

            let resolve_reload_path = |st: &AppState, mic_mode: bool| -> String {
                if mic_mode {
                    if let Some(ref dev) = st.selected_audio_device {
                        crate::state::mic_uri_for_device(dev)
                    } else {
                        "".to_string()
                    }
                } else if st.playlist_index < st.playlist.len() {
                    st.playlist[st.playlist_index].clone()
                } else {
                    st.song_title.clone()
                }
            };

            let device_change = state.audio_device_change_request.take();
            if let Some(new_device) = device_change {
                state.selected_audio_device = Some(new_device.clone());
                let reload_path = resolve_reload_path(&state, active_mic);

                if !reload_path.is_empty() || active_mic {
                    if active_mic && !state.playlist.is_empty() {
                        std::sync::Arc::make_mut(&mut state.playlist)[0] = reload_path.clone();
                    }
                    state.load_request = Some(reload_path);
                    resume_pos_on_reload = Some(state.current_seconds);
                    state.osd_text = Some(if active_mic {
                        format!("Audio Input: {}", new_device)
                    } else {
                        format!("Audio Output: {}", new_device)
                    });
                    if !cli_upmix_override {
                        state.surround_upmix.user_overridden = false;
                    }
                    state.osd_timer = 3.0;
                }
            }

            if let Some(pt_enabled) = state.passthrough_change_request.take() {
                let reload_path = resolve_reload_path(&state, active_mic);

                if (!reload_path.is_empty() && state.file_loaded) || active_mic {
                    state.load_request = Some(reload_path);
                    resume_pos_on_reload = Some(state.current_seconds);
                    reconcile_passthrough_change_osd(&mut state, pt_enabled);
                    state.osd_timer = 3.0;
                }
            }

            if let Some(osd_msg) = state.audio_topology_reload_request.take() {
                let reload_path = resolve_reload_path(&state, active_mic);

                if (!reload_path.is_empty() && state.file_loaded) || active_mic {
                    state.load_request = Some(reload_path);
                    resume_pos_on_reload = Some(state.current_seconds);
                    if !cli_upmix_override {
                        state.surround_upmix.user_overridden = false;
                    }
                    state.osd_text = Some(osd_msg);
                    state.osd_timer = 3.0;
                }
            }

            if state.audio_device_lost {
                state.audio_device_lost = false;
                let reload_path = resolve_reload_path(&state, active_mic);

                if !reload_path.is_empty() || active_mic {
                    state.load_request = Some(reload_path);
                    resume_pos_on_reload = Some(state.current_seconds);
                    if !cli_upmix_override {
                        state.surround_upmix.user_overridden = false;
                    }
                    state.osd_text = Some("Audio Device Reconnected".to_string());
                    state.osd_timer = 3.0;
                }
            }

            if state.track_ended && state.load_request.is_none() {
                if state.playlist_index + 1 < state.playlist.len() {
                    state.track_ended = false;
                    state.playlist_index += 1;
                    state.load_request = Some(state.playlist[state.playlist_index].clone());
                } else {
                    state.is_paused = true;
                }
            }

            if resume_pos_on_reload.is_none() && state.load_request.is_some() && !cli_upmix_override
            {
                state.surround_upmix.user_overridden = false;
            }
            state.sync_rt_control();
            (state.load_request.take(), resume_pos_on_reload)
        };

        if let Some(path) = load_path {
            let is_mic = crate::state::is_mic_uri(&path)
                || ((is_mic_launch || app_state.lock().unwrap().is_mic_input) && path.is_empty());
            {
                let mut state = app_state.lock().unwrap();
                let preferred_dev = crate::state::device_from_mic_uri(&path).map(|s| s.to_string());
                state.set_mic_input_mode(is_mic, preferred_dev);
            }
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
                    state.video_frame_rx = None;
                    state.free_video_frame_tx = None;
                    state.has_video_stream = false;
                    state.video_info = None;
                    state.video_mode = 0;
                    state.stats.bitstream_active = false;
                    state.stats.mpegh_bitstream_active = false;
                    state.stats.mpegh_carrier_info.clear();
                    state.stats.audio_buffer_fill_pct = 0.0;
                    state.stats.video_buffer_fill_pct = 0.0;
                    state.stats.video_decode_us = 0.0;
                    state.stats.video_hw_decoder.clear();
                    state.stats.video_fps = 0.0;
                    state.stats.video_bitrate_kbps = None;
                    state.stats.audio_bitrate_kbps = None;
                    state.track_ended = false;
                    let _ = state.rt_control.take_track_ended();
                    if !state.show_surround_eq {
                        state.tui_center_view = None;
                    }
                    state.sync_rt_control();
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
                    let _ = state.rt_control.take_track_ended();
                    if let Some(pos) = resume_pos_on_reload
                        && pos > 0.1
                    {
                        state.current_seconds = pos;
                        state.seek_request = Some(pos);
                        state.seek_epoch += 1;
                    }
                    state.sync_rt_control();
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

        let term_area = {
            let now = Instant::now();
            let tui_dt = now
                .duration_since(last_frame)
                .as_secs_f32()
                .clamp(0.001, 0.1);
            last_frame = now;
            let time_scale = (tui_dt * 60.0).clamp(0.15, 3.0);
            let mut state = app_state.lock().unwrap();

            state.tui_uptime_seconds += tui_dt;
            state.tick_osd_timer(tui_dt);

            if state.is_paused || state.track_ended {
                state.raw_channel_vus.fill(0.0);
                state.raw_spectrum_data.fill(0.0);
                state.gpu_spectrum_data.fill(0.0);
            }

            let dt_sec = tui_dt.clamp(0.002, 0.05);
            let vu_decay = 10.0_f32.powf(-20.0 * dt_sec / 20.0);
            let peak_decay = 10.0_f32.powf(-8.6 * dt_sec / 20.0);

            // Smoothing physics for VUs (logarithmic dB inertia matching GUI)
            if state.channel_vus.len() != state.raw_channel_vus.len() {
                state.channel_vus = vec![0.0; state.raw_channel_vus.len()];
            }
            for i in 0..state.raw_channel_vus.len() {
                if state.raw_channel_vus[i] > state.channel_vus[i] {
                    state.channel_vus[i] = state.raw_channel_vus[i];
                } else {
                    let decayed = state.channel_vus[i] * vu_decay;
                    state.channel_vus[i] =
                        if decayed < 1e-4 { 0.0 } else { decayed }.max(state.raw_channel_vus[i]);
                }
            }

            // Decay peaks and apply gravity
            if state.peak_vus.len() != state.channel_vus.len() {
                state.peak_vus = vec![0.0; state.channel_vus.len()];
            }
            for i in 0..state.channel_vus.len() {
                let decayed_peak = state.peak_vus[i] * peak_decay;
                state.peak_vus[i] = if decayed_peak < 1e-4 {
                    0.0
                } else {
                    decayed_peak
                };
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

            // Scroll spectrum history while active (reuse front allocation to avoid heap churn)
            if state.file_loaded && !state.is_paused && !state.track_ended {
                let mut recycled = if state.spectrum_history.len() >= 120 {
                    state.spectrum_history.pop_front().unwrap_or_default()
                } else {
                    Vec::with_capacity(state.spectrum_data.len())
                };
                recycled.clear();
                recycled.extend_from_slice(&state.spectrum_data);
                state.spectrum_history.push_back(recycled);
            }

            let completed = terminal.draw(|f| ui::draw(f, &state))?;
            completed.area
        };

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            loop {
                match event::read()? {
                    CEvent::Key(key) => {
                        let mut state = app_state.lock().unwrap();
                        if handle_tui_key_event(&mut state, key) {
                            drop(state);
                            drop(active_stream);
                            return Ok(());
                        }
                        state.sync_rt_control();
                    }
                    CEvent::Mouse(mouse) => {
                        let mut state = app_state.lock().unwrap();
                        handle_tui_mouse_event(&mut state, mouse, term_area);
                        state.sync_rt_control();
                    }
                    _ => {}
                }
                if !crossterm::event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        } else {
            // Wait out the remainder of the frame tick if woken up early by mouse movement
            let rem = tick_rate.saturating_sub(last_tick.elapsed());
            if !rem.is_zero() && !crossterm::event::poll(rem)? {
                last_tick = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppState, Args, KeyCode, Parser, apply_gamepad_stick_panel_resize, clamp_panel_split_ratio,
        clamp_window_geometry_to_monitor, extract_rusttracker_shortcut_appids,
        handle_tui_key_event, handle_tui_mouse_event, provision_steam_shortcut_artwork_in_root,
        upsert_rusttracker_shortcut_in_vdf,
    };
    use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

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

    #[test]
    fn test_window_geometry_preserves_full_steamdeck_1280x800_display() {
        let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
            clamp_window_geometry_to_monitor(1280, 800, 0, 0, 1.0, 1280.0, 800.0);

        assert_eq!((w, h), (1280.0, 800.0));
        assert_eq!((min_w, min_h), (1024.0, 600.0));
        assert_eq!((pos_x, pos_y), (0, 0));
    }

    #[test]
    fn test_gamepad_stick_resizes_panel_split_with_left_or_right_stick() {
        let mut state = AppState::new("test.mod".to_string());
        state.file_loaded = true;
        state.panel_split_ratio = 0.5;

        // Left stick UP (+1.0) shrinks top panel (moves split bar up)
        assert!(apply_gamepad_stick_panel_resize(&mut state, 1.0, 0.0, 0.1));
        assert!((state.panel_split_ratio - 0.4).abs() < 1e-5);

        // Right stick DOWN (-1.0) expands top panel (moves split bar down)
        assert!(apply_gamepad_stick_panel_resize(&mut state, 0.0, -1.0, 0.2));
        assert!((state.panel_split_ratio - 0.6).abs() < 1e-5);

        // Stick inside deadzone (<= 0.1) does not change ratio
        assert!(!apply_gamepad_stick_panel_resize(
            &mut state, 0.05, -0.09, 0.1
        ));
        assert!((state.panel_split_ratio - 0.6).abs() < 1e-5);

        // Moving stick while HUD is hidden restores HUD visibility
        state.show_hud = false;
        state.video_mode = 3;
        assert!(apply_gamepad_stick_panel_resize(&mut state, 0.5, 0.0, 0.1));
        assert!(state.show_hud);
        assert_eq!(state.video_mode, 0);

        // Modal dialogs suppress background panel resizing
        state.show_vis_picker = true;
        let before = state.panel_split_ratio;
        assert!(!apply_gamepad_stick_panel_resize(&mut state, 1.0, 1.0, 0.2));
        assert_eq!(state.panel_split_ratio, before);
    }

    #[test]
    fn test_clamp_panel_split_ratio_allows_full_steamdeck_range() {
        // On 1280x800 Steam Deck, min_h = 120px corresponds to [0.15, 0.85]
        assert!((clamp_panel_split_ratio(0.10, 800.0) - 0.15).abs() < 1e-5);
        assert!((clamp_panel_split_ratio(0.90, 800.0) - 0.85).abs() < 1e-5);
        assert!((clamp_panel_split_ratio(0.20, 800.0) - 0.20).abs() < 1e-5);
        assert!((clamp_panel_split_ratio(0.80, 800.0) - 0.80).abs() < 1e-5);
    }

    #[test]
    fn test_extract_and_provision_steam_shortcut_artwork() {
        // Construct a realistic binary shortcuts.vdf with 1 RustTracker entry (appid 0xEA304690 = 3929032336)
        // and 1 unrelated entry (appid 0x81234567)
        let mut vdf = Vec::new();
        vdf.extend_from_slice(b"\x00shortcuts\x00");
        vdf.extend_from_slice(b"\x000\x00");
        vdf.extend_from_slice(b"\x02appid\x00\x90\x46\x30\xea");
        vdf.extend_from_slice(b"\x01AppName\x00RustTracker-SteamDeck-GamePad.AppImage\x00");
        vdf.extend_from_slice(
            b"\x01Exe\x00\"/home/deck/Downloads/RustTracker-SteamDeck-GamePad.AppImage\"\x00",
        );
        vdf.extend_from_slice(b"\x01icon\x00\x00");
        vdf.extend_from_slice(b"\x00tags\x00\x08\x08");
        vdf.extend_from_slice(b"\x001\x00");
        vdf.extend_from_slice(b"\x02appid\x00\x67\x45\x23\x81");
        vdf.extend_from_slice(b"\x01AppName\x00OtherApp\x00");
        vdf.extend_from_slice(b"\x01Exe\x00\"/usr/bin/other\"\x00\x08");
        vdf.push(0x08);

        let appids = extract_rusttracker_shortcut_appids(&vdf);
        assert_eq!(appids, vec![3929032336]);

        let tmp_root =
            std::env::temp_dir().join(format!("rusttracker_steamgrid_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_root);
        let config_dir = tmp_root.join("userdata/8232340/config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("shortcuts.vdf"), &vdf).unwrap();

        let steamgrid_dir = std::path::Path::new("assets/steamgrid");
        let icon_png = std::path::Path::new("assets/icons/hicolor/512x512/apps/rusttracker.png");
        let count =
            provision_steam_shortcut_artwork_in_root(&tmp_root, steamgrid_dir, Some(icon_png));
        assert_eq!(count, 1);

        // Verify shortcuts.vdf had AppName normalized to "RustTracker" and icon populated
        let updated_vdf = std::fs::read(config_dir.join("shortcuts.vdf")).unwrap();
        assert!(
            updated_vdf
                .windows(b"\x01AppName\x00RustTracker\x00".len())
                .any(|w| w == b"\x01AppName\x00RustTracker\x00")
        );
        assert!(
            updated_vdf
                .windows(b"rusttracker.png\x00".len())
                .any(|w| w == b"rusttracker.png\x00")
        );

        assert!(config_dir.join("grid/3929032336.jpg").exists());
        assert!(config_dir.join("grid/3929032336p.jpg").exists());
        assert!(config_dir.join("grid/3929032336_hero.jpg").exists());
        assert!(config_dir.join("grid/3929032336_logo.png").exists());
        assert!(config_dir.join("grid/3929032336_icon.png").exists());
        assert!(
            tmp_root
                .join("appcache/librarycache/3929032336/header.jpg")
                .exists()
        );
        assert!(
            tmp_root
                .join("appcache/librarycache/3929032336_header.jpg")
                .exists()
        );
        assert!(
            tmp_root
                .join("appcache/librarycache/3929032336/library_600x900.jpg")
                .exists()
        );

        // Verify upsert into an empty shortcuts.vdf creates a valid RustTracker entry
        let (created_vdf, created_id) = upsert_rusttracker_shortcut_in_vdf(
            &[],
            "/home/deck/Downloads/RustTracker-SteamDeck.AppImage",
            "/home/deck/Downloads/",
            "/home/deck/.local/share/icons/hicolor/512x512/apps/rusttracker.png",
        );
        assert_eq!(created_id, 3929032336);
        assert_eq!(
            extract_rusttracker_shortcut_appids(&created_vdf),
            vec![3929032336]
        );
        let _ = std::fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn test_cli_surround_upmix_flag_and_state_toggle() {
        let args_long = Args::try_parse_from(["rusttracker", "--surround-upmix"]).unwrap();
        assert!(args_long.surround_upmix);

        let args_alias = Args::try_parse_from(["rusttracker", "--upmix"]).unwrap();
        assert!(args_alias.surround_upmix);

        let args_default = Args::try_parse_from(["rusttracker"]).unwrap();
        assert!(!args_default.surround_upmix);

        assert!(crate::state::SurroundUpmixConfig::default().enabled);
        assert!(!crate::state::SurroundUpmixConfig::default().user_overridden);
        assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(2, 2));
        assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(6, 6));
        assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(8, 8));
        assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(12, 12));
        assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
            2, 6
        ));
        assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
            2, 8
        ));
        assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
            2, 12
        ));
        assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
            6, 8
        ));
        assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
            6, 12
        ));

        let mut state = AppState::new("Test Track".to_string());
        state.surround_upmix.enabled =
            crate::state::SurroundUpmixConfig::should_enable_by_default(2, 2);
        state.force_stereo_downmix = true;
        state.show_hud = false;
        state.video_mode = 3;
        state.file_loaded = true;

        // 1. On a standard stereo (2ch) device, enabling surround upmix must NOT change num_channels
        state.hardware_channels = 2;
        state.num_channels = 2;
        state.toggle_surround_upmix();
        let is_active_2 = state.surround_upmix.is_active_for_hardware(2);
        assert!(state.surround_upmix.enabled);
        assert!(!is_active_2);
        assert!(!state.force_stereo_downmix);
        assert_eq!(
            state.num_channels, 2,
            "Stereo (2ch) output device must remain 2ch when surround upmix is enabled"
        );

        state.toggle_surround_upmix();
        assert!(!state.surround_upmix.enabled);
        assert_eq!(state.num_channels, 2);

        // 2. On a Bed-Only 5.1 (6ch) device, enabling surround upmix sets num_channels to 6 (not 12)
        state.hardware_channels = 6;
        state.show_hud = false;
        state.video_mode = 3;
        state.toggle_surround_upmix();
        let is_active_6 = state.surround_upmix.is_active_for_hardware(6);
        let has_height_6 = crate::state::SurroundUpmixConfig::has_height_channels(6);
        assert!(state.surround_upmix.enabled);
        assert!(is_active_6);
        assert!(!has_height_6);
        assert_eq!(state.num_channels, 6);
        assert!(state.show_hud);
        assert_eq!(state.video_mode, 0);

        state.toggle_surround_upmix();
        assert!(!state.surround_upmix.enabled);
        assert_eq!(state.num_channels, 2);

        // 3. On a Bed + Height 7.1.4 (12ch) device, enabling surround upmix while bitstream passthrough
        // is active exits bitstream mode and sets num_channels to 12
        state.hardware_channels = 12;
        state.passthrough_enabled = true;
        state.stats.bitstream_active = true;
        state.raw_channel_vus = vec![0.1; 6];
        state.set_surround_upmix(true);
        let has_height_12 = crate::state::SurroundUpmixConfig::has_height_channels(12);
        assert!(state.surround_upmix.enabled);
        assert!(has_height_12);
        assert!(!state.passthrough_enabled);
        assert_eq!(state.passthrough_change_request, Some(false));
        assert_eq!(state.num_channels, 12);

        // Verify disabling surround upmix restores intrinsic channel count from raw_channel_vus
        state.set_surround_upmix(false);
        assert!(!state.surround_upmix.enabled);
        assert_eq!(state.num_channels, 6);
    }

    #[test]
    fn test_cumulative_seek_osd_and_reset() {
        let mut state = AppState::new("Test Track".to_string());

        // Single jump forward shows +5.0s
        state.record_seek_osd(5.0);
        assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));
        assert!((state.osd_timer - 2.0).abs() < 1e-5);

        // Repeated jump forward before 2.0s timer expires accumulates
        state.tick_osd_timer(0.5);
        state.record_seek_osd(5.0);
        assert_eq!(state.osd_text.as_deref(), Some("+10.0s"));
        assert!((state.osd_timer - 2.0).abs() < 1e-5);

        state.tick_osd_timer(0.8);
        state.record_seek_osd(5.0);
        assert_eq!(state.osd_text.as_deref(), Some("+15.0s"));
        assert!((state.osd_timer - 2.0).abs() < 1e-5);

        // After 2.0 seconds, the OSD expires and resets cumulative time
        state.tick_osd_timer(2.0);
        assert_eq!(state.osd_text, None);
        assert_eq!(state.osd_timer, 0.0);
        assert_eq!(state.cumulative_scrub, 0.0);

        // Next jump starts fresh at +5.0s
        state.record_seek_osd(5.0);
        assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));

        // Jumping in the opposite direction resets to the new direction and accumulates backward
        state.record_seek_osd(-5.0);
        assert_eq!(state.osd_text.as_deref(), Some("-5.0s"));
        state.record_seek_osd(-5.0);
        assert_eq!(state.osd_text.as_deref(), Some("-10.0s"));

        // Non-seek OSD message interrupts accumulation so subsequent seek starts fresh
        state.osd_text = Some("Paused".to_string());
        state.osd_timer = 2.0;
        state.record_seek_osd(5.0);
        assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));

        // Frame-stepping resets cumulative seconds
        state.record_seek_osd(1.0 / 60.0);
        assert_eq!(state.osd_text.as_deref(), Some("+1 Frame"));
        state.record_seek_osd(5.0);
        assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));
    }

    #[test]
    fn test_cli_mpegh_flag_and_state_toggle() {
        let args_mpegh = Args::try_parse_from(["rusttracker", "--mpegh"]).unwrap();
        assert!(args_mpegh.mpegh);
        assert!(!args_mpegh.no_mpegh);

        let args_mpegh_714 = Args::try_parse_from(["rusttracker", "--mpegh-714"]).unwrap();
        assert!(args_mpegh_714.mpegh);

        let args_no_mpegh = Args::try_parse_from(["rusttracker", "--no-mpegh"]).unwrap();
        assert!(args_no_mpegh.no_mpegh);
        assert!(!args_no_mpegh.mpegh);

        let default_cfg = crate::state::SurroundUpmixConfig::default();
        assert!(default_cfg.mpegh_bitstream);
        assert!(!default_cfg.mpegh_user_overridden);

        let mut state = AppState::new("Test MPEG-H".to_string());
        state.file_loaded = true;
        state.set_mpegh_bitstream(false);
        assert!(!state.surround_upmix.mpegh_bitstream);
        assert!(state.surround_upmix.mpegh_user_overridden);
        assert_eq!(state.passthrough_change_request, Some(false));

        state.passthrough_change_request = None;
        state.toggle_mpegh_bitstream();
        assert!(state.surround_upmix.mpegh_bitstream);
        assert!(state.surround_upmix.mpegh_user_overridden);
        assert!(state.surround_upmix.enabled);
        assert_eq!(state.passthrough_change_request, Some(false));
    }

    #[test]
    fn test_middle_panel_defaults_to_pattern_view_and_flips_to_eq() {
        let mut state = AppState::new("Test Tracker.mod".to_string());
        state.file_loaded = true;
        state.source_channels = 2;
        state.hardware_channels = 8;

        // Surround upmix is enabled and active by default on surround hardware,
        // while the middle panel defaults to the Tracker Pattern / Heatmap viewer (`show_surround_eq == false`).
        assert!(state.surround_upmix.enabled);
        assert!(state.surround_upmix.is_active_for(2, 8));
        assert!(!state.show_surround_eq);
        assert!(!state.render_snapshot().show_surround_eq);

        // Flip middle panel to Surround EQ
        state.toggle_show_surround_eq();
        assert!(state.show_surround_eq);
        assert!(state.render_snapshot().show_surround_eq);

        // Flip back to Tracker Pattern / Heatmap viewer
        state.toggle_show_surround_eq();
        assert!(!state.show_surround_eq);

        // Disabling surround upmix while viewing EQ returns to Pattern View
        state.set_show_surround_eq(true);
        assert!(state.show_surround_eq);
        state.set_surround_upmix(false);
        assert!(!state.show_surround_eq);

        // Verify middle panel title bar buttons and consolidated upmix/layout labels in crates/rt-render/src/engine.rs
        let engine_src = include_str!("../crates/rt-render/src/engine.rs");
        assert!(engine_src.contains("🎚 Surround EQ"));
        assert!(engine_src.contains("📊 Pattern View"));
        assert!(!engine_src.contains("🎹 Piano Roll"));
        assert!(!engine_src.contains("🔊 SURROUND UPMIX: ON"));
        assert!(!engine_src.contains("11.1 (7.1.4): ON"));
        assert_eq!(crate::state::SurroundUpmixConfig::short_tag(8), "7.1");
        assert!(!crate::state::SurroundUpmixConfig::has_height_channels(8));
    }

    #[test]
    fn test_tui_key_event_controls_and_multitrack_solo() {
        let mut state = AppState::new("MultiTrack.mkv".to_string());
        state.file_loaded = true;
        state.duration_seconds = 100.0;
        state.current_seconds = 20.0;
        state.audio_tracks = (0..3)
            .map(|id| crate::state::AudioTrackInfo {
                id,
                title: format!("Track {}", id + 1),
                codec: "FLAC".to_string(),
                channels: 2,
                sample_rate: 48000,
                language: None,
                is_atmos: false,
            })
            .collect();
        state.active_audio_tracks = vec![0, 1, 2];
        state.selected_audio_track = 0;

        // 1. Pressing '1' solos Track 1 from the initial full mix
        let should_quit = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
        );
        assert!(!should_quit);
        assert_eq!(state.selected_audio_track, 0);
        assert_eq!(state.active_audio_tracks, vec![0]);
        assert_eq!(state.audio_track_request, Some(0));

        // 2. Pressing '4' (num_tracks + 1) restores the full multi-track mix
        state.audio_mix_request = None;
        let _ = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
        );
        assert_eq!(state.active_audio_tracks, vec![0, 1, 2]);
        assert!(state.audio_mix_request.is_some());

        // 3. View cycling with 'v' and Tab
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::Heatmap
        );
        let _ = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
        );
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::SurroundEq
        );
        let _ = handle_tui_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::Heatmap
        );

        // 4. Bitstream passthrough shortcut ('b') resets SurroundEq/upmix on Linux/Windows and stays honest elsewhere
        let _ = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
        );
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::SurroundEq
        );
        let _ = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
        );
        if cfg!(any(target_os = "windows", target_os = "linux")) {
            assert!(state.passthrough_enabled);
            assert!(!state.surround_upmix.enabled);
            assert_eq!(
                state.effective_tui_center_view(),
                crate::state::TuiCenterView::Heatmap
            );
            assert_eq!(state.passthrough_change_request, Some(true));
        } else {
            assert!(!state.passthrough_enabled);
            assert!(state.passthrough_change_request.is_none());
        }

        // 5. Split ratio adjustment with '[' and ']'
        let initial_ratio = state.panel_split_ratio;
        let _ = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
        );
        assert!(state.panel_split_ratio > initial_ratio);
        let _ = handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
        );
        assert!((state.panel_split_ratio - initial_ratio).abs() < 1e-4);

        // 6. Quit with Esc, 'q', and Ctrl+C
        assert!(handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));
        assert!(handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        ));
        assert!(handle_tui_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ));
    }

    #[test]
    fn test_tui_mouse_seek_and_center_panel_click() {
        let mut state = AppState::new("MouseTest.flac".to_string());
        state.file_loaded = true;
        state.duration_seconds = 200.0;
        let term_area = ratatui::layout::Rect::new(0, 0, 120, 40);
        let meta_len = crate::ui::build_track_info_lines(&state, 30).len();
        let layout = crate::ui::compute_tui_layout(term_area, &state, meta_len);

        // Click at the midpoint of the timeline bar seeks to ~50% (100s)
        let inner_x = layout.timeline_rect.x + layout.timeline_rect.width / 2;
        let inner_y = layout.timeline_rect.y;
        let click_seek = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inner_x,
            row: inner_y,
            modifiers: KeyModifiers::NONE,
        };
        handle_tui_mouse_event(&mut state, click_seek, term_area);
        let seek_target = state
            .seek_request
            .expect("Expected seek_request from timeline click");
        assert!(
            (seek_target - 100.0).abs() < 5.0,
            "Expected seek near 100.0s, got {seek_target}"
        );

        // Click inside the top-center panel cycles the center view
        let center_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.center_rect.x + 2,
            row: layout.center_rect.y + 2,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::Heatmap
        );
        handle_tui_mouse_event(&mut state, center_click, term_area);
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::SurroundEq
        );
    }

    #[test]
    fn test_cli_args_bench_windowed_fullscreen_and_help() {
        use clap::CommandFactory;

        // Bare --bench (alone or after positional file) defaults to Some(300)
        let bare_only = Args::try_parse_from(["rusttracker", "--bench"])
            .expect("Bare --bench alone must parse");
        assert_eq!(bare_only.bench, Some(300));

        let bare_trailing = Args::try_parse_from(["rusttracker", "song.mod", "--bench"])
            .expect("Trailing bare --bench must parse");
        assert_eq!(bare_trailing.bench, Some(300));
        assert_eq!(bare_trailing.file, vec!["song.mod".to_string()]);

        // Normalized CLI args also allow `--bench song.mod` without swallowing the filename
        let bare_leading = Args::try_parse_from(super::normalize_cli_args([
            std::ffi::OsString::from("rusttracker"),
            std::ffi::OsString::from("--bench"),
            std::ffi::OsString::from("song.mod"),
        ]))
        .expect("Normalized `--bench song.mod` must parse");
        assert_eq!(bare_leading.bench, Some(300));
        assert_eq!(bare_leading.file, vec!["song.mod".to_string()]);

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let non_utf8_path = std::ffi::OsString::from_vec(vec![
                b's', b'o', b'n', b'g', 0xff, b'.', b'm', b'o', b'd',
            ]);
            let normalized = super::normalize_cli_args([
                std::ffi::OsString::from("rusttracker"),
                std::ffi::OsString::from("--bench"),
                non_utf8_path.clone(),
            ]);
            assert_eq!(normalized[1], std::ffi::OsString::from("--bench=300"));
            assert_eq!(normalized[2], non_utf8_path);
        }

        // Explicit --bench <N> parses N
        let explicit_bench = Args::try_parse_from(super::normalize_cli_args([
            std::ffi::OsString::from("rusttracker"),
            std::ffi::OsString::from("--bench"),
            std::ffi::OsString::from("120"),
            std::ffi::OsString::from("song.mod"),
        ]))
        .expect("Explicit --bench 120 must parse");
        assert_eq!(explicit_bench.bench, Some(120));
        assert_eq!(explicit_bench.file, vec!["song.mod".to_string()]);

        // --windowed parses cleanly alone, and conflicts with --fullscreen
        let win_args =
            Args::try_parse_from(["rusttracker", "--windowed"]).expect("--windowed must parse");
        assert!(win_args.windowed);
        assert!(!win_args.fullscreen);
        assert!(
            Args::try_parse_from(["rusttracker", "--windowed", "--fullscreen"]).is_err(),
            "--windowed and --fullscreen must conflict"
        );

        // Removed placebo --gpu-fft flag must be rejected
        assert!(
            Args::try_parse_from(["rusttracker", "--gpu-fft"]).is_err(),
            "Removed placebo --gpu-fft flag must be rejected"
        );

        // Every flag in --help must have a non-empty description
        let mut cmd = Args::command();
        let help_text = cmd.render_help().to_string();
        for flag in [
            "--tui",
            "--mic",
            "--fullscreen",
            "--windowed",
            "--vis",
            "--list-vis",
            "--bench",
            "--uncapped",
            "--stereo",
            "--passthrough",
            "--surround-upmix",
            "--no-surround-upmix",
            "--mpegh",
            "--no-mpegh",
            "--register-steam",
        ] {
            assert!(
                help_text.contains(flag),
                "Help output missing expected flag {flag}"
            );
        }
        assert!(!help_text.contains("--gpu-fft"));
    }

    #[test]
    fn test_readme_cli_flags_and_env_vars_match_args_parser() {
        use clap::CommandFactory;

        let readme = include_str!("../README.md");
        let flags_header = "### Command-Line Flags";
        let env_header = "### Environment Variables";
        let after_flags = readme
            .split_once(flags_header)
            .expect("README.md must contain `### Command-Line Flags`")
            .1;
        let (flags_section, after_env_split) = after_flags
            .split_once(env_header)
            .expect("README.md must contain `### Environment Variables` after Command-Line Flags");
        let env_section = after_env_split
            .split_once("\n---")
            .map(|(s, _)| s)
            .unwrap_or(after_env_split);

        let sample_for_placeholder = |token: &str| -> &'static str {
            match token.trim() {
                "id" | "frames" => "120",
                "name" => "spectrum",
                "true" => "true",
                "false" => "false",
                _ => "test_value",
            }
        };

        let run_normalized_parse = |argv: &[&str]| -> Result<Args, clap::Error> {
            let os_args: Vec<std::ffi::OsString> =
                argv.iter().map(std::ffi::OsString::from).collect();
            Args::try_parse_from(super::normalize_cli_args(os_args))
        };

        // Extract every backticked `--...` span in `### Command-Line Flags`
        let mut documented_spans: Vec<String> = Vec::new();
        for line in flags_section.lines() {
            let mut rest = line;
            while let Some(start) = rest.find('`') {
                let after_open = &rest[start + 1..];
                let Some(end) = after_open.find('`') else {
                    break;
                };
                let span = after_open[..end].trim();
                if span.starts_with("--") {
                    documented_spans.push(span.to_string());
                }
                rest = &after_open[end + 1..];
            }
        }

        assert!(
            documented_spans.len() >= 14,
            "Expected at least 14 documented CLI flag spans in README.md, found {}",
            documented_spans.len()
        );

        let mut seen_flag_names = std::collections::HashSet::new();
        for span in &documented_spans {
            let parts: Vec<&str> = span.split_whitespace().collect();
            let flag_name = parts[0];
            seen_flag_names.insert(flag_name.to_string());

            match parts.get(1).copied() {
                None => {
                    // Bare `--flag` invocation must always parse cleanly alone
                    let parsed_alone = run_normalized_parse(&["rusttracker", flag_name])
                        .unwrap_or_else(|e| {
                            panic!("Documented flag `{span}` failed standalone parse: {e}")
                        });
                    assert!(parsed_alone.file.is_empty());

                    // Every bare flag (including bare `--vis`) with no value placeholder must NOT
                    // swallow a trailing positional media filename.
                    let parsed_with_file =
                        run_normalized_parse(&["rusttracker", flag_name, "song.mod"])
                            .unwrap_or_else(|e| {
                                panic!("Documented flag `{span}` failed parse before filename: {e}")
                            });
                    assert_eq!(
                        parsed_with_file.file,
                        vec!["song.mod".to_string()],
                        "Documented flag `{span}` swallowed `song.mod` instead of leaving it as sole positional file"
                    );
                }
                Some(arg_spec) if arg_spec.starts_with('<') && arg_spec.ends_with('>') => {
                    let inner = &arg_spec[1..arg_spec.len() - 1];
                    for alt in inner.split('|') {
                        let sample = sample_for_placeholder(alt);
                        let parsed = run_normalized_parse(&[
                            "rusttracker",
                            flag_name,
                            sample,
                            "song.mod",
                        ])
                        .unwrap_or_else(|e| {
                            panic!(
                                "Documented flag `{span}` with value `{sample}` failed parse: {e}"
                            )
                        });
                        assert_eq!(
                            parsed.file,
                            vec!["song.mod".to_string()],
                            "Documented flag `{span}` with value `{sample}` treated value as positional file! (got {:?})",
                            parsed.file
                        );
                    }
                }
                Some(arg_spec) if arg_spec.starts_with('[') && arg_spec.ends_with(']') => {
                    let inner = &arg_spec[1..arg_spec.len() - 1];
                    // Optional value omitted (both standalone and followed by positional filename)
                    let parsed_bare = run_normalized_parse(&["rusttracker", flag_name])
                        .unwrap_or_else(|e| {
                            panic!("Documented optional flag `{span}` failed bare parse: {e}")
                        });
                    assert!(parsed_bare.file.is_empty());

                    let parsed_omitted_with_file =
                        run_normalized_parse(&["rusttracker", flag_name, "song.mod"])
                            .unwrap_or_else(|e| {
                                panic!(
                                    "Documented optional flag `{span}` failed bare parse before filename: {e}"
                                )
                            });
                    assert_eq!(
                        parsed_omitted_with_file.file,
                        vec!["song.mod".to_string()],
                        "Documented optional flag `{span}` swallowed `song.mod` when value omitted"
                    );

                    // Optional value provided explicitly
                    for alt in inner.split('|') {
                        let sample = sample_for_placeholder(alt);
                        let parsed_explicit =
                            run_normalized_parse(&["rusttracker", flag_name, sample, "song.mod"])
                                .unwrap_or_else(|e| {
                                    panic!(
                                        "Documented optional flag `{span}` with `{sample}` failed parse: {e}"
                                    )
                                });
                        assert_eq!(parsed_explicit.file, vec!["song.mod".to_string()]);
                    }
                }
                Some(literal_val) => {
                    // Literal example such as `--vis list`
                    let parsed = run_normalized_parse(&["rusttracker", flag_name, literal_val])
                        .unwrap_or_else(|e| {
                            panic!("Documented invocation `{span}` failed parse: {e}")
                        });
                    assert!(parsed.file.is_empty());
                }
            }
        }

        // Reverse check: every `--long` flag and visible alias in `Args` must have non-empty
        // `--help` text and be documented in README.md
        let cmd = Args::command();
        for arg in cmd.get_arguments() {
            if arg.get_id() != "help" && arg.get_id() != "version" {
                assert!(
                    arg.get_help()
                        .is_some_and(|h| !h.to_string().trim().is_empty()),
                    "CLI argument `{}` in `struct Args` is missing `--help` description",
                    arg.get_id()
                );
            }
            let Some(long) = arg.get_long() else {
                continue;
            };
            if long == "help" || long == "version" {
                continue;
            }
            let expected = format!("--{long}");
            assert!(
                seen_flag_names.contains(&expected),
                "CLI flag `{expected}` exists in `struct Args` (`src/main.rs`) but is missing from `README.md` Command-Line Flags"
            );
            if let Some(aliases) = arg.get_visible_aliases() {
                for alias in aliases {
                    let expected_alias = format!("--{alias}");
                    assert!(
                        seen_flag_names.contains(&expected_alias),
                        "Visible CLI alias `{expected_alias}` exists in `struct Args` but is missing from `README.md` Command-Line Flags"
                    );
                }
            }
        }

        // Verify all user-facing RUSTTRACKER_* environment variables are documented in README.md,
        // and scan source files so no undocumented RUSTTRACKER_* env var can sneak in.
        for env_var in [
            "RUSTTRACKER_FPS_LIMIT",
            "RUSTTRACKER_PROFILE",
            "RUSTTRACKER_PRESENT_MODE",
            "RUSTTRACKER_PIPELINE_CACHE",
            "RUSTTRACKER_CACHE_DIR",
            "RUSTTRACKER_WAYLAND",
            "RUSTTRACKER_X11",
        ] {
            assert!(
                env_section.contains(env_var),
                "Environment variable `{env_var}` missing from `README.md` Environment Variables section"
            );
        }

        for (file_label, src_text) in [
            ("src/main.rs", include_str!("main.rs")),
            (
                "crates/rt-render/src/engine.rs",
                include_str!("../crates/rt-render/src/engine.rs"),
            ),
        ] {
            for needle in ["env::var(\"RUSTTRACKER_", "env::var_os(\"RUSTTRACKER_"] {
                let mut search = src_text;
                while let Some(pos) = search.find(needle) {
                    let after_quote = &search[pos + needle.len() - "RUSTTRACKER_".len()..];
                    if let Some(end_q) = after_quote.find('"') {
                        let var_name = &after_quote[..end_q];
                        // `RUSTTRACKER_APPDIR` is an internal AppImage mount path set by AppRun
                        if var_name != "RUSTTRACKER_APPDIR" {
                            assert!(
                                env_section.contains(var_name),
                                "Undocumented `{var_name}` read in `{file_label}` but missing from `README.md` Environment Variables"
                            );
                        }
                        search = &after_quote[end_q + 1..];
                    } else {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn test_cli_stereo_and_no_surround_upmix_flags() {
        // 1. `--stereo` forces 2.0 downmix, disables surround upmix + passthrough, and marks user_overridden
        let stereo_args =
            Args::try_parse_from(["rusttracker", "--stereo", "track.mkv"]).expect("--stereo");
        assert!(stereo_args.stereo);
        assert!(super::has_cli_upmix_override(&stereo_args));
        assert_eq!(stereo_args.file, vec!["track.mkv".to_string()]);
        let mut stereo_state = AppState::new("track.mkv".to_string());
        stereo_state.passthrough_enabled = true;
        super::apply_cli_audio_flags(&mut stereo_state, &stereo_args);
        assert!(stereo_state.force_stereo_downmix);
        assert!(!stereo_state.surround_upmix.enabled);
        assert!(stereo_state.surround_upmix.user_overridden);
        assert!(!stereo_state.passthrough_enabled);
        assert_eq!(
            stereo_state.osd_text.as_deref(),
            Some("🎧 Stereo Downmix (2.0): Forced")
        );

        // `--stereo --no-mpegh` and `--stereo --no-surround-upmix` must keep the stereo OSD
        let stereo_no_mpegh =
            Args::try_parse_from(["rusttracker", "--stereo", "--no-mpegh", "track.mkv"])
                .expect("--stereo --no-mpegh");
        let mut stereo_no_mpegh_state = AppState::new("track.mkv".to_string());
        super::apply_cli_audio_flags(&mut stereo_no_mpegh_state, &stereo_no_mpegh);
        assert!(stereo_no_mpegh_state.force_stereo_downmix);
        assert!(!stereo_no_mpegh_state.surround_upmix.mpegh_bitstream);
        assert_eq!(
            stereo_no_mpegh_state.osd_text.as_deref(),
            Some("🎧 Stereo Downmix (2.0): Forced")
        );

        // Runtime `set_force_stereo_downmix` preserves stereo OSD across `reconcile_passthrough_change_osd`
        let mut runtime_stereo = AppState::new("track.mkv".to_string());
        runtime_stereo.file_loaded = true;
        runtime_stereo.hardware_channels = 8;
        runtime_stereo.passthrough_enabled = true;
        runtime_stereo.set_force_stereo_downmix(true);
        assert!(runtime_stereo.force_stereo_downmix);
        assert!(!runtime_stereo.passthrough_enabled);
        assert!(!runtime_stereo.surround_upmix.enabled);
        assert!(runtime_stereo.surround_upmix.user_overridden);
        assert_eq!(runtime_stereo.passthrough_change_request, Some(false));
        super::reconcile_passthrough_change_osd(&mut runtime_stereo, false);
        assert_eq!(
            runtime_stereo.osd_text.as_deref(),
            Some("🎧 Stereo Downmix (2.0): Forced")
        );
        runtime_stereo.set_force_stereo_downmix(false);
        assert!(!runtime_stereo.force_stereo_downmix);
        assert!(!runtime_stereo.surround_upmix.user_overridden);
        super::reconcile_passthrough_change_osd(&mut runtime_stereo, false);
        assert_eq!(
            runtime_stereo.osd_text.as_deref(),
            Some("🎧 Stereo Downmix: OFF")
        );

        // 2. `--no-surround-upmix` and `--no-upmix` disable automatic upmixing cleanly
        let no_upmix_args =
            Args::try_parse_from(["rusttracker", "--no-surround-upmix", "song.flac"])
                .expect("--no-surround-upmix");
        assert!(no_upmix_args.no_surround_upmix);
        assert!(!no_upmix_args.surround_upmix);
        assert!(super::has_cli_upmix_override(&no_upmix_args));
        assert_eq!(no_upmix_args.file, vec!["song.flac".to_string()]);

        let no_upmix_alias =
            Args::try_parse_from(["rusttracker", "--no-upmix", "song.flac"]).expect("--no-upmix");
        assert!(no_upmix_alias.no_surround_upmix);

        let mut no_upmix_state = AppState::new("song.flac".to_string());
        assert!(no_upmix_state.surround_upmix.enabled);
        super::apply_cli_audio_flags(&mut no_upmix_state, &no_upmix_args);
        assert!(!no_upmix_state.surround_upmix.enabled);
        assert!(no_upmix_state.surround_upmix.user_overridden);
        assert!(!no_upmix_state.force_stereo_downmix);

        // 3. `--mpegh` preserves `has_cli_upmix_override` and keeps 11.1 OSD; `--no-surround-upmix --mpegh`
        // allows native >8ch MPEG-H bitstreaming without re-enabling upmix
        let mpegh_only_args =
            Args::try_parse_from(["rusttracker", "--mpegh", "song.flac"]).expect("--mpegh");
        assert!(super::has_cli_upmix_override(&mpegh_only_args));
        let mut mpegh_only_state = AppState::new("song.flac".to_string());
        super::apply_cli_audio_flags(&mut mpegh_only_state, &mpegh_only_args);
        if cfg!(any(target_os = "windows", target_os = "linux")) {
            assert_eq!(
                mpegh_only_state.osd_text.as_deref(),
                Some("🔊 11.1 (7.1.4 Spatial Audio / MPEG-H 3D): ON")
            );
        }

        // `--surround-upmix` updates pre-load "Armed" OSD to "ON (<badge>)" once >2ch stream opens
        let upmix_args = Args::try_parse_from(["rusttracker", "--surround-upmix", "song.flac"])
            .expect("--surround-upmix");
        let mut upmix_state = AppState::new("song.flac".to_string());
        super::apply_cli_audio_flags(&mut upmix_state, &upmix_args);
        upmix_state.hardware_channels = 8;
        upmix_state.source_channels = 2;
        super::reconcile_post_load_cli_audio_osd(&mut upmix_state, &upmix_args);
        assert_eq!(
            upmix_state.osd_text.as_deref(),
            Some("🔊 Surround Upmix: ON (7.1 Surround — Bed Only)")
        );

        let native_mpegh_args = Args::try_parse_from([
            "rusttracker",
            "--no-surround-upmix",
            "--mpegh",
            "bed714.wav",
        ])
        .expect("--no-surround-upmix --mpegh");
        assert!(super::has_cli_upmix_override(&native_mpegh_args));
        let mut native_mpegh_state = AppState::new("bed714.wav".to_string());
        super::apply_cli_audio_flags(&mut native_mpegh_state, &native_mpegh_args);
        assert!(!native_mpegh_state.surround_upmix.enabled);
        if cfg!(any(target_os = "windows", target_os = "linux")) {
            assert!(native_mpegh_state.surround_upmix.mpegh_bitstream);
        }

        // 4. Mutually contradictory audio mode flags are rejected by clap
        for conflicting in [
            &["rusttracker", "--stereo", "--passthrough"][..],
            &["rusttracker", "--stereo", "--surround-upmix"][..],
            &["rusttracker", "--stereo", "--upmix"][..],
            &["rusttracker", "--stereo", "--mpegh"][..],
            &["rusttracker", "--surround-upmix", "--no-surround-upmix"][..],
            &["rusttracker", "--upmix", "--no-upmix"][..],
            &["rusttracker", "--mpegh", "--no-mpegh"][..],
            &["rusttracker", "--passthrough", "--surround-upmix"][..],
            &["rusttracker", "--passthrough", "--mpegh"][..],
        ] {
            assert!(
                Args::try_parse_from(conflicting).is_err(),
                "Expected conflicting flags {conflicting:?} to be rejected"
            );
        }
    }

    #[test]
    fn test_cli_vis_and_bench_normalization_and_x11_wayland_precedence() {
        let parse_norm = |argv: &[&str]| -> Result<Args, clap::Error> {
            let os_args: Vec<std::ffi::OsString> =
                argv.iter().map(std::ffi::OsString::from).collect();
            Args::try_parse_from(super::normalize_cli_args(os_args))
        };

        // Bare `--vis` before a media path, URL, or `--` lists visualizers without swallowing the file
        for (argv, expected_file) in [
            (&["rusttracker", "--vis", "song.mod"][..], "song.mod"),
            (
                &["rusttracker", "--vis", "audio_tests/ive_got_the_power.mod"][..],
                "audio_tests/ive_got_the_power.mod",
            ),
            (
                &["rusttracker", "--vis", "https://example.com/live.ogg"][..],
                "https://example.com/live.ogg",
            ),
            (
                &["rusttracker", "--vis", "--", "-song.mod"][..],
                "-song.mod",
            ),
        ] {
            let parsed = parse_norm(argv).unwrap_or_else(|e| panic!("{argv:?} failed: {e}"));
            assert_eq!(parsed.vis.as_deref(), Some(""));
            assert_eq!(parsed.file, vec![expected_file.to_string()]);
        }

        // Explicit visualizer names/IDs and unknown non-path tokens still bind to `--vis`
        let vis_named = parse_norm(&["rusttracker", "--vis", "spectrum", "song.mod"]).unwrap();
        assert_eq!(vis_named.vis.as_deref(), Some("spectrum"));
        assert_eq!(vis_named.file, vec!["song.mod".to_string()]);

        let vis_unknown = parse_norm(&["rusttracker", "--vis", "nonexistent_vis", "song.mod"])
            .expect("unknown vis token");
        assert_eq!(vis_unknown.vis.as_deref(), Some("nonexistent_vis"));
        assert_eq!(vis_unknown.file, vec!["song.mod".to_string()]);

        // `--bench` before `--` defaults to 300, whereas overflowing numeric frame counts fail cleanly
        let bench_sep = parse_norm(&["rusttracker", "--bench", "--", "-song.mod"]).unwrap();
        assert_eq!(bench_sep.bench, Some(300));
        assert_eq!(bench_sep.file, vec!["-song.mod".to_string()]);
        assert!(
            parse_norm(&["rusttracker", "--bench", "99999999999999999999", "song.mod"]).is_err(),
            "Overflowing numeric `--bench` count must fail clap u32 validation instead of becoming a filename"
        );

        // `RUSTTRACKER_X11=1` always overrides `RUSTTRACKER_WAYLAND=1`
        assert!(super::should_prefer_x11_backend(
            Some("1"),
            Some("1"),
            true,
            true
        ));
        // Explicit `RUSTTRACKER_WAYLAND=1` or default with active `WAYLAND_DISPLAY` uses native Wayland
        assert!(!super::should_prefer_x11_backend(
            Some("1"),
            None,
            true,
            true
        ));
        assert!(!super::should_prefer_x11_backend(None, None, true, true));
        assert!(!super::should_prefer_x11_backend(None, None, true, false));
        // `RUSTTRACKER_WAYLAND=0` / `false` forces X11 when DISPLAY is available
        assert!(super::should_prefer_x11_backend(
            Some("0"),
            None,
            true,
            true
        ));
        assert!(super::should_prefer_x11_backend(
            Some("false"),
            None,
            true,
            true
        ));
        // Falls back to X11 when WAYLAND_DISPLAY is absent and DISPLAY is present
        assert!(super::should_prefer_x11_backend(None, None, false, true));
        assert!(!super::should_prefer_x11_backend(None, None, false, false));
    }

    #[test]
    fn test_gamepad_l2_r2_scrub_no_hud_collision_and_l3_r3_bindings() {
        let mut state = AppState::new("ScrubTest.mod".to_string());
        state.file_loaded = true;
        state.duration_seconds = 120.0;
        state.current_seconds = 60.0;
        state.show_hud = true;
        state.hardware_channels = 8;
        state.source_channels = 2;

        // 1. Pulling L2 (analog + digital LeftTrigger2 event from gilrs) scrubs backward and NEVER hides HUD
        assert!(
            !super::apply_gamepad_playback_button(&mut state, gilrs::Button::LeftTrigger2),
            "Digital LeftTrigger2 must not trigger a button action"
        );
        assert!(state.show_hud, "LeftTrigger2 must never toggle HUD");
        assert!(super::apply_gamepad_trigger_scrub(
            &mut state,
            0.8,
            0.0,
            1.0 / 60.0
        ));
        assert!(
            state.current_seconds < 60.0,
            "L2 analog pull must scrub backward"
        );
        assert!(state.seek_request.is_some());
        assert!(state.show_hud, "L2 analog scrub must leave HUD visible");

        // 2. Pulling R2 (analog + digital RightTrigger2 event) scrubs forward and NEVER hides HUD
        let after_l2 = state.current_seconds;
        assert!(!super::apply_gamepad_playback_button(
            &mut state,
            gilrs::Button::RightTrigger2
        ));
        assert!(super::apply_gamepad_trigger_scrub(
            &mut state,
            0.0,
            1.0,
            1.0 / 60.0
        ));
        assert!(state.current_seconds > after_l2);
        assert!(state.show_hud);

        // 3. Deadzone and modal/unloaded guards prevent spurious scrubbing or seek OSDs
        state.seek_request = None;
        assert!(!super::apply_gamepad_trigger_scrub(
            &mut state,
            0.02,
            0.0,
            1.0 / 60.0
        ));
        assert!(state.seek_request.is_none());
        state.show_vis_picker = true;
        assert!(!super::apply_gamepad_trigger_scrub(
            &mut state,
            1.0,
            0.0,
            1.0 / 60.0
        ));
        assert!(!super::apply_gamepad_playback_button(
            &mut state,
            gilrs::Button::DPadRight
        ));
        state.show_vis_picker = false;

        let mut unloaded = AppState::new(String::new());
        for seek_btn in [
            gilrs::Button::DPadLeft,
            gilrs::Button::DPadRight,
            gilrs::Button::LeftTrigger,
            gilrs::Button::RightTrigger,
        ] {
            assert!(
                !super::apply_gamepad_playback_button(&mut unloaded, seek_btn),
                "Unloaded state must ignore {seek_btn:?} seek without false OSD"
            );
            assert!(unloaded.seek_request.is_none());
            assert!(unloaded.osd_text.is_none());
        }

        // 4. L3 (LeftThumb) toggles HUD visibility cleanly
        state.video_mode = 3;
        assert!(super::apply_gamepad_playback_button(
            &mut state,
            gilrs::Button::LeftThumb
        ));
        assert!(!state.show_hud);
        assert_eq!(state.video_mode, 0);
        assert!(super::apply_gamepad_playback_button(
            &mut state,
            gilrs::Button::LeftThumb
        ));
        assert!(state.show_hud);

        // 5. R3 (RightThumb) toggles Surround Upmix cleanly without affecting L3 HUD role
        let initial_upmix = state.surround_upmix.enabled;
        assert!(super::apply_gamepad_playback_button(
            &mut state,
            gilrs::Button::RightThumb
        ));
        assert_eq!(state.surround_upmix.enabled, !initial_upmix);
        assert!(super::apply_gamepad_playback_button(
            &mut state,
            gilrs::Button::RightThumb
        ));
        assert_eq!(state.surround_upmix.enabled, initial_upmix);

        // 6. Verify env_flag_enabled handles truthy/falsy/empty values accurately
        assert!(super::env_flag_enabled(Some("1")));
        assert!(super::env_flag_enabled(Some("true")));
        assert!(super::env_flag_enabled(Some("TRUE")));
        assert!(super::env_flag_enabled(Some(" 1 ")));
        assert!(!super::env_flag_enabled(Some("0")));
        assert!(!super::env_flag_enabled(Some("false")));
        assert!(!super::env_flag_enabled(Some("FALSE")));
        assert!(!super::env_flag_enabled(Some("")));
        assert!(!super::env_flag_enabled(Some("   ")));
        assert!(!super::env_flag_enabled(None));

        // 7. Verify UI overlays, README, VIS_REVIEW, and THIRD_PARTY_NOTICES match live ground truth
        let engine_src = include_str!("../crates/rt-render/src/engine.rs");
        assert!(!engine_src.contains("\"L2\", \"Toggle HUD\""));
        assert!(!engine_src.contains("Some(\"L2\"), \"Toggle HUD\""));
        assert!(!engine_src.contains("\"q / esc\""));
        assert!(!engine_src.contains("Surround Upmix & EQ"));

        let state_src = include_str!("../crates/rt-core/src/state.rs");
        let shader_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
        let mut total_wgsl = 0usize;
        let mut standalone_wgsl = 0usize;
        for entry in std::fs::read_dir(&shader_dir).expect("read src/shaders") {
            let path = entry.expect("dir entry").path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("wgsl") {
                total_wgsl += 1;
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with('_') {
                    standalone_wgsl += 1;
                    assert!(
                        engine_src.contains(name) || state_src.contains(name),
                        "Standalone shader {name} in src/shaders/ is not referenced in engine.rs or state.rs"
                    );
                }
            }
        }
        let shared_wgsl = total_wgsl - standalone_wgsl;
        assert_eq!(shared_wgsl, 2);
        assert_eq!(standalone_wgsl, 30);
        assert_eq!(total_wgsl, 32);

        let vis_count = crate::state::VISUALIZERS.len();
        let vis_review = include_str!("../VIS_REVIEW.md");
        assert!(vis_review.contains("Document Status Note:"));
        assert!(vis_review.contains(&format!("{vis_count}` registered visualizers")));
        assert!(vis_review.contains(&format!("{total_wgsl}` `.wgsl` files")));
        assert!(vis_review.contains(&format!(
            "{standalone_wgsl}` standalone Naga-validated shader modules"
        )));
        assert!(vis_review.contains(&format!(
            "- {standalone_wgsl} standalone WGSL shader modules + {shared_wgsl} shared include headers (`_common.wgsl`, `_glyph_font.wgsl`) = {total_wgsl} `.wgsl` files"
        )));
        assert!(!vis_review.contains("51` `.wgsl` files"));
        assert!(!vis_review.contains("49 standalone WGSL"));

        let notices = include_str!("../THIRD_PARTY_NOTICES.txt");
        assert!(notices.contains("MPEG-H 3D Audio Low Complexity Profile Encoder"));
        assert!(!notices.contains("Baseline Profile Encoder"));
        assert!(notices.contains("[third_party/libmpeghe/LICENSE]"));
        assert!(notices.contains("[third_party/libmpeghe/LICENSE2]"));
    }

    #[test]
    fn test_license_notices_and_packaging_manifests_invariants() {
        let root_license = include_str!("../LICENSE").replace("\r\n", "\n");
        let notices = include_str!("../THIRD_PARTY_NOTICES.txt").replace("\r\n", "\n");
        let libmpeghe_license =
            include_str!("../third_party/libmpeghe/LICENSE").replace("\r\n", "\n");
        let libmpeghe_license2 =
            include_str!("../third_party/libmpeghe/LICENSE2").replace("\r\n", "\n");

        // 1. Verify GPLv3 §7 Additional Permission and verbatim Ittiam LICENSE + LICENSE2 reproduction
        assert!(root_license.contains(
            "Additional Permission Under GNU GPL Version 3 Section 7 (`libmpeghe` Exception)"
        ));
        assert!(root_license.contains("THIRD_PARTY_NOTICES.txt"));
        assert!(notices.contains("MPEG-H 3D Audio Low Complexity Profile Encoder"));
        assert!(!notices.contains("Baseline Profile Encoder"));
        assert!(
            notices.contains(libmpeghe_license.trim()),
            "THIRD_PARTY_NOTICES.txt must reproduce third_party/libmpeghe/LICENSE verbatim"
        );
        assert!(
            notices.contains(libmpeghe_license2.trim()),
            "THIRD_PARTY_NOTICES.txt must reproduce third_party/libmpeghe/LICENSE2 verbatim"
        );

        // 2. Verify all 6 distribution manifests bundle LICENSE and THIRD_PARTY_NOTICES.txt
        let cargo_toml = include_str!("../Cargo.toml");
        assert!(
            cargo_toml.contains(r#"dest = "/usr/share/licenses/rusttracker/LICENSE""#)
                && cargo_toml.contains(
                    r#"dest = "/usr/share/licenses/rusttracker/THIRD_PARTY_NOTICES.txt""#
                ),
            "Cargo.toml [package.metadata.generate-rpm] must bundle LICENSE and THIRD_PARTY_NOTICES.txt"
        );
        assert!(
            cargo_toml.contains(r#"["LICENSE", "usr/share/doc/rusttracker/", "644"]"#)
                && cargo_toml.contains(
                    r#"["THIRD_PARTY_NOTICES.txt", "usr/share/doc/rusttracker/", "644"]"#
                ),
            "Cargo.toml [package.metadata.deb] must bundle LICENSE and THIRD_PARTY_NOTICES.txt"
        );

        let repackage = include_str!("../repackage_appimage.sh");
        assert!(
            repackage
                .contains("cp LICENSE THIRD_PARTY_NOTICES.txt AppDir/usr/share/doc/rusttracker/")
                && repackage.contains(
                    "cp LICENSE THIRD_PARTY_NOTICES.txt AppDir/usr/share/licenses/rusttracker/"
                ),
            "repackage_appimage.sh must install LICENSE and THIRD_PARTY_NOTICES.txt into AppDir"
        );

        let installer_iss = include_str!("../installer.iss");
        assert!(
            installer_iss.contains(r#"Excludes: "LICENSE,THIRD_PARTY_NOTICES.txt""#),
            "installer.iss must exclude LICENSE and THIRD_PARTY_NOTICES.txt from release\\* wildcard to prevent duplicate entries"
        );
        assert!(
            installer_iss.contains(r#"Source: "LICENSE"; DestDir: "{app}""#)
                && installer_iss.contains(r#"Source: "THIRD_PARTY_NOTICES.txt"; DestDir: "{app}""#),
            "installer.iss must bundle LICENSE and THIRD_PARTY_NOTICES.txt into {{app}}"
        );

        let win_yml = include_str!("../.github/workflows/windows-build.yml");
        assert!(
            win_yml.contains("cp LICENSE THIRD_PARTY_NOTICES.txt release/"),
            "windows-build.yml must stage LICENSE and THIRD_PARTY_NOTICES.txt into release/"
        );

        let macos_yml = include_str!("../.github/workflows/macos.yml");
        assert!(
            macos_yml.contains(
                "cp assets/soundfont.sf2 LICENSE THIRD_PARTY_NOTICES.txt RustTracker.app/Contents/Resources/"
            ),
            "macos.yml must bundle LICENSE and THIRD_PARTY_NOTICES.txt into RustTracker.app/Contents/Resources/"
        );

        let android_gradle = include_str!("../android/app/build.gradle.kts");
        assert!(
            android_gradle
                .contains(r#"File(rootDir, "LICENSE"), File(rootDir, "THIRD_PARTY_NOTICES.txt")"#)
                && android_gradle.contains("addGeneratedSourceDirectory(")
                && android_gradle.contains("SyncLicenseAssetsTask::outputDir"),
            "android/app/build.gradle.kts must wire SyncLicenseAssetsTask via AGP addGeneratedSourceDirectory"
        );
    }

    #[test]
    fn test_phase3_workspace_crates_and_openmpt_direct_ffi_invariants() {
        let root_toml = include_str!("../Cargo.toml");
        assert!(
            root_toml.contains("[workspace]"),
            "Root Cargo.toml must define [workspace]"
        );
        for member in [
            "crates/rt-core",
            "crates/rt-decode",
            "crates/rt-audio",
            "crates/rt-render",
            "crates/rt-tui",
        ] {
            assert!(
                root_toml.contains(member),
                "Workspace member {member} missing from root Cargo.toml"
            );
            let sub_toml_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(member)
                .join("Cargo.toml");
            let sub_toml = std::fs::read_to_string(&sub_toml_path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", sub_toml_path.display()));
            assert!(
                !sub_toml.contains("openmpt =") && !sub_toml.contains("openmpt-sys"),
                "{} must not depend on unmaintained openmpt / openmpt-sys crates",
                sub_toml_path.display()
            );
        }
        assert!(
            !root_toml.contains("openmpt =") && !root_toml.contains("openmpt-sys"),
            "Root Cargo.toml must not depend on unmaintained openmpt / openmpt-sys crates"
        );

        // Ensure Linux active-sink monitoring in rt-audio never spawns wpctl/pactl subprocesses
        let audio_src = include_str!("../crates/rt-audio/src/audio.rs");
        let audio_prod = audio_src.split("#[cfg(test)]").next().unwrap_or(audio_src);
        assert!(
            !audio_prod.contains("Command::new(\"wpctl\")")
                && !audio_prod.contains("Command::new(\"pactl\")"),
            "crates/rt-audio/src/audio.rs must use non-spawning in-process Linux audio topology monitoring"
        );

        // Ensure zero hidden dead_code/unused/deprecated allow attributes across workspace Rust files
        for rel_file in [
            "src/lib.rs",
            "src/main.rs",
            "src/android.rs",
            "crates/rt-core/src/state.rs",
            "crates/rt-core/src/biquad.rs",
            "crates/rt-core/src/devices.rs",
            "crates/rt-core/src/ipc.rs",
            "crates/rt-core/src/lyrics.rs",
            "crates/rt-core/src/playlist.rs",
            "crates/rt-core/src/touch.rs",
            "crates/rt-decode/src/openmpt_ffi.rs",
            "crates/rt-decode/src/android_video.rs",
            "crates/rt-decode/src/iso.rs",
            "crates/rt-decode/src/mpegh.rs",
            "crates/rt-audio/src/audio.rs",
            "crates/rt-audio/src/bitstream.rs",
            "crates/rt-render/src/engine.rs",
            "crates/rt-tui/src/ui.rs",
        ] {
            let full_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel_file);
            let content = std::fs::read_to_string(&full_path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", full_path.display()));
            let prod = content.split("#[cfg(test)]").next().unwrap_or(&content);
            for forbidden in ["allow(dead_code", "allow(unused", "allow(deprecated"] {
                assert!(
                    !prod.contains(forbidden),
                    "Forbidden warning suppression `{forbidden}` found in {rel_file}"
                );
            }
        }
    }

    #[test]
    fn test_vis_review_flip_water_and_package_metadata_truthfulness() {
        let vis_review = include_str!("../VIS_REVIEW.md");
        let flip_water_wgsl = include_str!("shaders/flip_water_compute.wgsl");
        let water_splash_wgsl = include_str!("shaders/vis_water_splash.wgsl");

        assert!(flip_water_wgsl.contains("const SOR_OMEGA: f32 = 1.28;"));
        assert!(flip_water_wgsl.contains("fn cs_pressure_0_to_1("));
        assert!(flip_water_wgsl.contains("fn cs_pressure_1_to_0("));
        assert!(flip_water_wgsl.contains("fn cs_g2p_advect_splat("));
        assert!(flip_water_wgsl.contains("splash_compact.active_indices[slot] = p_idx;"));
        assert!(water_splash_wgsl.contains("fn vs_splash("));
        assert!(water_splash_wgsl.contains("splash_compact.active_indices[inst_id]"));

        assert!(vis_review.contains("`SOR_OMEGA = 1.28`"));
        assert!(vis_review.contains("`cs_pressure_0_to_1` / `cs_pressure_1_to_0`"));
        assert!(vis_review.contains("`cs_g2p_advect_splat`"));
        assert!(vis_review.contains("`splash_compact.active_indices`"));
        assert!(vis_review.contains("`vs_splash`"));
        assert!(!vis_review.contains("`flip_pressure`"));
        assert!(!vis_review.contains(r"$\omega = 1.5$"));

        let cargo_toml = include_str!("../Cargo.toml");
        assert!(cargo_toml.contains(r#"authors = ["Soddentrough"]"#));
        assert!(cargo_toml.contains(r#"copyright = "2026, Soddentrough""#));
        assert!(cargo_toml.contains(
            r#"description = "High-Performance WGPU Audio, Video & Tracker Visualizer""#
        ));
        assert!(cargo_toml.contains(
            r#"extended-description = "A high-performance, real-time audio visualizer, video player, and tracker module player.""#
        ));
        assert!(!cargo_toml.contains("Vulkan Tracker Module Visualizer"));

        let metainfo = include_str!("../com.soddentrough.rusttracker.metainfo.xml");
        assert!(metainfo.contains(
            "<summary>High-Performance WGPU Audio, Video &amp; Tracker Visualizer</summary>"
        ));
        assert!(!metainfo.contains("Vulkan Tracker Module Visualizer"));

        let desktop = include_str!("../rusttracker.desktop");
        assert!(
            desktop.contains("Comment=High-Performance WGPU Audio, Video & Tracker Visualizer")
        );
    }
}
