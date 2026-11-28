#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

pub(crate) use clap::Parser;
use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};
use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
};

use rusttracker::{audio, engine, ipc, lyrics, playlist, state, touch, ui};

mod cli;
mod gui_runner;
mod steam;
mod tui_runner;

use cli::{
    Args, absolutize_ipc_paths, apply_cli_audio_flags, has_cli_upmix_override,
    matches_visualizer_spec, normalize_cli_args, reconcile_post_load_cli_audio_osd,
};
#[cfg(target_os = "macos")]
use gui_runner::MACOS_OPEN_FILES_TX;
use gui_runner::run_gui;
pub(crate) use state::AppState;
use steam::register_steam_shortcut_cli;
use tui_runner::{Tui, run_tui};

#[cfg(test)]
pub(crate) use cli::{
    env_flag_enabled, reconcile_passthrough_change_osd, should_prefer_x11_backend,
};
#[cfg(test)]
pub(crate) use crossterm::event::KeyCode;
#[cfg(test)]
pub(crate) use gui_runner::{
    advance_playlist_on_track_ended, apply_gamepad_playback_button,
    apply_gamepad_stick_panel_resize, apply_gamepad_stick_variant_cycle,
    apply_gamepad_trigger_scrub, apply_ipc_file_paths_to_state, apply_picked_file_paths_to_state,
    clamp_panel_split_ratio, clamp_window_geometry_to_monitor,
};
#[cfg(test)]
pub(crate) use steam::{
    extract_rusttracker_shortcut_appids, provision_steam_shortcut_artwork_in_root,
    upsert_rusttracker_shortcut_in_vdf,
};
#[cfg(test)]
pub(crate) use tui_runner::{handle_tui_key_event, handle_tui_mouse_event};

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
    let should_list_vis = args.list_vis
        || args.vis.as_deref().is_some_and(|s| {
            matches!(s.trim().to_ascii_lowercase().as_str(), "" | "list" | "help")
        });
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

    // Single-instance forwarding: if not in TUI/bench/utility mode and files were specified,
    // absolutize relative paths (so a running instance with a different CWD resolves them)
    // and forward to the running instance.
    if !args.tui && args.bench.is_none() && !args.file.is_empty() {
        let forwarded_paths = absolutize_ipc_paths(&args.file);
        if crate::ipc::try_forward_to_existing_instance(&forwarded_paths) {
            println!("[ipc] Forwarded file(s) to existing RustTracker instance.");
            return Ok(());
        }
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
        state.set_audio_devices(
            crate::audio::get_available_audio_devices(false),
            crate::audio::get_available_audio_devices(true),
        );
        state.playlist = std::sync::Arc::new(args.file.clone());
        apply_cli_audio_flags(&mut state, &args);
        state.auto_vis_guard = args.auto_vis_guard;
        state.playlist_index = 0;

        let mut history_changed = false;
        for path in &args.file {
            let lower = path.to_ascii_lowercase();
            if (lower.starts_with("http://") || lower.starts_with("https://"))
                && !state.url_history.iter().any(|(u, _)| u == path)
            {
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

#[cfg(test)]
mod tests;
