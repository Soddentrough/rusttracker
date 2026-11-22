use crate::audio;
use crate::cli::{
    Args, has_cli_upmix_override, reconcile_passthrough_change_osd, should_prefer_x11_backend,
};
use crate::engine::{EngineAction, VulkanEngine};
use crate::state::AppState;
use crate::steam::auto_provision_steam_artwork;
use std::{
    error::Error,
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

struct DesktopRuntime {
    egui_state: egui_winit::State,
    engine: VulkanEngine,
    window: Arc<winit::window::Window>,
}

#[cfg(target_os = "macos")]
pub(crate) static MACOS_OPEN_FILES_TX: std::sync::OnceLock<crossbeam_channel::Sender<Vec<String>>> =
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
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(16),
                    ));
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
pub(crate) fn clamp_window_geometry_to_monitor(
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
            // 'B' or Circle -> Close topmost open modal or Toggle Stats
            if state.show_vis_picker {
                state.show_vis_picker = false;
            } else if state.is_url_dialog_open {
                state.is_url_dialog_open = false;
            } else if state.show_help {
                state.show_help = false;
            } else if state.show_surround_eq {
                state.show_surround_eq = false;
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

fn cached_fps_limit() -> Option<f32> {
    static FPS_LIMIT: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *FPS_LIMIT.get_or_init(|| {
        std::env::var("RUSTTRACKER_FPS_LIMIT")
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .filter(|&fps| fps.is_finite() && fps > 0.0)
    })
}

fn sync_keep_awake(
    keep_awake: &mut Option<keepawake::KeepAwake>,
    active_mode: &mut Option<(bool, bool)>,
    needs_keepawake: bool,
    keep_display_on: bool,
) {
    let desired = needs_keepawake.then_some((true, keep_display_on));
    if *active_mode == desired {
        return;
    }
    *keep_awake = if needs_keepawake {
        keepawake::Builder::default()
            .display(keep_display_on)
            .idle(true)
            .create()
            .ok()
    } else {
        None
    };
    *active_mode = desired;
}

fn apply_picked_file_paths(
    state: &mut AppState,
    paths: Vec<String>,
    initial_dir: &mut std::path::PathBuf,
    window: &winit::window::Window,
) {
    if paths.is_empty() {
        return;
    }
    if let Some(parent) = std::path::Path::new(&paths[0]).parent() {
        *initial_dir = parent.to_path_buf();
    }
    let expanded = crate::playlist::expand_input_paths(&paths);
    let append = state.append_to_playlist;
    if append && !state.playlist.is_empty() && !state.is_mic_input {
        std::sync::Arc::make_mut(&mut state.playlist).extend(expanded);
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

fn apply_ipc_file_paths(
    state: &mut AppState,
    raw_paths: Vec<String>,
    window: &winit::window::Window,
) {
    if raw_paths.is_empty() {
        window.set_minimized(false);
        window.focus_window();
        window.request_redraw();
        return;
    }
    let expanded = crate::playlist::expand_input_paths(&raw_paths);
    if expanded.is_empty() {
        return;
    }
    if !state.file_loaded || state.playlist.is_empty() || state.is_mic_input {
        state.set_mic_input_mode(false, None);
        state.playlist = std::sync::Arc::new(expanded);
        state.playlist_index = 0;
        state.load_request = Some(state.playlist[0].clone());
        state.file_loaded = true;
    } else {
        let first_new_idx = state.playlist.len();
        std::sync::Arc::make_mut(&mut state.playlist).extend(expanded.clone());
        state.playlist_index = first_new_idx;
        state.load_request = Some(state.playlist[first_new_idx].clone());
        state.is_paused = false;
        let file_name = std::path::Path::new(&expanded[0])
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if expanded.len() == 1 {
            state.osd_text = Some(format!("Now Playing: {}", file_name));
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

pub(crate) async fn run_gui(
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
                    &crate::engine::gpu_adapter_profile_from_wgpu(&engine.adapter_info),
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
    let mut keep_awake_mode: Option<(bool, bool)> = None;
    sync_keep_awake(
        &mut keep_awake,
        &mut keep_awake_mode,
        is_fullscreen,
        is_fullscreen,
    );
    let mut is_first_frame = true;
    let mut last_mouse_scrub_seek = Instant::now();
    let mut touch_controller = crate::touch::TouchGestureController::new();
    let mut last_touch_scrub_seek = Instant::now();

    let mut gilrs = gilrs::Gilrs::new().ok();

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
    let mut redraw_requested = true;
    let mut egui_gamepad_events: Vec<egui::Event> = Vec::new();
    let mut ui_state = AppState::new(String::new());

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
                            let touch_ev = crate::engine::touch_event_from_winit(touch);
                            if let Some(gesture) = touch_controller.handle_touch(
                                &touch_ev,
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
                                        redraw_requested = true;
                                    }
                                    crate::touch::TouchGesture::ScrubEnd => {
                                        let mut st = app_state.lock().unwrap();
                                        st.finish_touch_scrub();
                                        window.request_redraw();
                                        redraw_requested = true;
                                    }
                                    crate::touch::TouchGesture::ScrubCancel => {
                                        let mut st = app_state.lock().unwrap();
                                        st.cancel_touch_scrub();
                                        window.request_redraw();
                                        redraw_requested = true;
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
                        redraw_requested = true;
                    } else if let WindowEvent::HoveredFileCancelled = event {
                        let mut state = app_state.lock().unwrap();
                        state.hovered_file = None;
                        state.drag_cursor_pos = None;
                        window.request_redraw();
                        redraw_requested = true;
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

                                    let frame_dt = if state.stats.video_fps.is_finite()
                                        && state.stats.video_fps >= 1.0
                                    {
                                        1.0 / state.stats.video_fps as f64
                                    } else {
                                        1.0 / 60.0
                                    };
                                    match keycode {
                                        WinitKeyCode::ArrowRight => {
                                            delta = step;
                                        }
                                        WinitKeyCode::ArrowLeft => {
                                            delta = -step;
                                        }
                                        WinitKeyCode::Comma => delta = -frame_dt,
                                        WinitKeyCode::Period => delta = frame_dt,
                                        _ => {}
                                    }

                                    let target = (state.current_seconds + delta)
                                        .clamp(0.0, state.duration_seconds);
                                    state.seek_request = Some(target);
                                    state.current_seconds = target;
                                    state.track_ended = false;
                                    let _ = state.rt_control.take_track_ended();
                                    clear_spectrum_history_in_place(&mut state);

                                    state.record_seek_osd(delta);
                                }
                            }
                            _ => {
                                if !kb_event.repeat {
                                    match keycode {
                                        WinitKeyCode::Escape => {
                                            let mut close_file_dialog = false;
                                            let mut exit_app = false;
                                            {
                                                let mut state = app_state.lock().unwrap();
                                                if state.show_vis_picker {
                                                    state.show_vis_picker = false;
                                                } else if state.is_url_dialog_open {
                                                    state.is_url_dialog_open = false;
                                                    window.set_ime_allowed(false);
                                                } else if state.show_help {
                                                    state.show_help = false;
                                                } else if state.show_surround_eq {
                                                    state.set_show_surround_eq(false);
                                                } else if state.is_file_picker_open {
                                                    state.is_file_picker_open = false;
                                                    close_file_dialog = true;
                                                } else {
                                                    exit_app = true;
                                                }
                                            }
                                            if close_file_dialog {
                                                file_dialog = egui_file_dialog::FileDialog::new()
                                                    .default_size([920.0, 560.0])
                                                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                                                    .initial_directory(initial_dir.clone())
                                                    .add_file_filter_extensions(
                                                        "Audio/Video Files",
                                                        crate::playlist::FILE_DIALOG_EXTENSIONS
                                                            .to_vec(),
                                                    )
                                                    .show_all_files_filter(true)
                                                    .default_file_filter("Audio/Video Files");
                                            }
                                            if exit_app {
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
                                            if !egui_ctx.egui_wants_keyboard_input() {
                                                let mut state = app_state.lock().unwrap();
                                                if !state.show_vis_picker
                                                    && !state.is_url_dialog_open
                                                    && !state.is_file_picker_open
                                                {
                                                    state.show_hud = !state.show_hud;
                                                    state.osd_text = Some(format!(
                                                        "HUD: {}",
                                                        if state.show_hud {
                                                            "Visible"
                                                        } else {
                                                            "Hidden"
                                                        }
                                                    ));
                                                    state.osd_timer = 2.0;
                                                }
                                            }
                                        }
                                        WinitKeyCode::KeyF => {
                                            let currently_fullscreen =
                                                window.fullscreen().is_some();
                                            is_fullscreen = !currently_fullscreen;
                                            if is_fullscreen {
                                                window.set_fullscreen(Some(
                                                    winit::window::Fullscreen::Borderless(None),
                                                ));
                                                sync_keep_awake(
                                                    &mut keep_awake,
                                                    &mut keep_awake_mode,
                                                    true,
                                                    true,
                                                );
                                            } else {
                                                window.set_fullscreen(None);
                                                constrain_window_to_monitor(
                                                    window,
                                                    Some(desired_logical_w),
                                                    Some(desired_logical_h),
                                                    true,
                                                    force_unit_scale,
                                                );
                                                sync_keep_awake(
                                                    &mut keep_awake,
                                                    &mut keep_awake_mode,
                                                    false,
                                                    false,
                                                );
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
                                            } else if state.visualizer_mode == 31 {
                                                state.room_volume_smoke = !state.room_volume_smoke;
                                                state.osd_text = Some(if state.room_volume_smoke {
                                                    "💨 Smoke Volume: ON".to_string()
                                                } else {
                                                    "💨 Smoke Volume: OFF".to_string()
                                                });
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
                                                let _ = state.rt_control.take_track_ended();
                                                state.seek_request = Some(0.0);
                                                clear_spectrum_history_in_place(&mut state);
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
                                            redraw_requested = true;
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
                        redraw_requested = true;
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
                            redraw_requested = false;
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

                            let fps_limit = if !is_uncapped && bench.is_none() {
                                cached_fps_limit()
                            } else {
                                None
                            };
                            let is_minimized = window.is_minimized() == Some(true);

                            let phase_timer = Instant::now();
                            {
                                let mut state = app_state.lock().unwrap();
                                state.current_fps = fps;
                                if bench.is_none() && !is_minimized {
                                    let target_fps =
                                        fps_limit.unwrap_or(state.target_fps.max(1) as f32);
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
                                let gamepad_input_active = is_game_mode || window.has_focus();
                                if let Some(ref mut gilrs) = gilrs {
                                    for (_id, gamepad) in
                                        gilrs.gamepads().filter(|_| gamepad_input_active)
                                    {
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
                                                if gamepad.is_pressed(gilrs::Button::RightTrigger2)
                                                {
                                                    1.0
                                                } else {
                                                    0.0
                                                }
                                            });
                                        if rt > max_rt {
                                            max_rt = rt;
                                        }
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

                            // Create a lightweight snapshot for render reusing `ui_state` heap
                            // capacities (skips ~1.3 MB of audio data) and drain gamepad events
                            let snapshot_timer = Instant::now();
                            let gamepad_events = {
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
                                state.render_snapshot_into(&mut ui_state);
                                std::mem::take(&mut egui_gamepad_events)
                            };
                            let state_copy = &ui_state;
                            let phase_snapshot_us = snapshot_timer.elapsed().as_micros() as f32;

                            // Update power management / idle sleep inhibition
                            let is_playing = !state_copy.is_paused
                                && state_copy.file_loaded
                                && !state_copy.track_ended;
                            let needs_keepawake = is_playing || is_fullscreen;
                            let keep_display_on = is_fullscreen
                                || state_copy.visualizer_mode != 0
                                || state_copy.video_frame_rx.is_some();
                            sync_keep_awake(
                                &mut keep_awake,
                                &mut keep_awake_mode,
                                needs_keepawake,
                                keep_display_on,
                            );

                            let mut actions: Vec<EngineAction> = Vec::new();
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
                                state_copy,
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
                                    actions = res;
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
                                let mut should_quit = false;
                                for action in actions {
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
                                            if let Some(target) = state.scrub_target_seconds.take()
                                            {
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
                                            let expanded =
                                                crate::playlist::expand_input_paths(&paths);
                                            if append
                                                && !state.playlist.is_empty()
                                                && state.file_loaded
                                                && !state.is_mic_input
                                            {
                                                if !expanded.is_empty() {
                                                    let count = expanded.len();
                                                    let first_name =
                                                        std::path::Path::new(&expanded[0])
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
                                                state.load_request =
                                                    Some(state.playlist[0].clone());
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
                                        EngineAction::ToggleRoomVolumeSmoke => {
                                            state.room_volume_smoke = !state.room_volume_smoke;
                                            state.osd_text = Some(if state.room_volume_smoke {
                                                "💨 Smoke Volume: ON".to_string()
                                            } else {
                                                "💨 Smoke Volume: OFF".to_string()
                                            });
                                            state.osd_timer = 2.0;
                                        }
                                        EngineAction::SetPassthrough(val) => {
                                            if cfg!(any(target_os = "windows", target_os = "linux"))
                                            {
                                                state.passthrough_enabled = val;
                                                if val {
                                                    state.surround_upmix.enabled = false;
                                                    state.force_stereo_downmix = false;
                                                    state.show_surround_eq = false;
                                                }
                                                state.osd_text = Some(if val {
                                                    "Bitstream Passthrough: ON (Exclusive)"
                                                        .to_string()
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
                                                    .unwrap_or_else(|| {
                                                        format!("Track {}", idx + 1)
                                                    });
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
                                            state.visualizer_mode =
                                                crate::state::VISUALIZERS[idx].id;
                                            state.osd_text = Some(
                                                crate::state::VISUALIZERS[idx].name.to_string(),
                                            );
                                            state.osd_timer = 2.0;
                                            state.show_vis_picker = false;
                                        }
                                        EngineAction::ToggleVisPicker => {
                                            state.show_vis_picker = !state.show_vis_picker;
                                            if state.show_vis_picker {
                                                state.vis_picker_cursor =
                                                    state.current_visualizer_idx;
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
                                            state.set_audio_devices(
                                                Vec::new(),
                                                crate::audio::get_available_audio_devices(true),
                                            );
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
                                            state.set_mic_input_mode(
                                                true,
                                                Some(device_name.clone()),
                                            );
                                            state.playlist =
                                                std::sync::Arc::new(vec![mic_uri.clone()]);
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
                                                let dev_name = crate::state::device_from_mic_uri(
                                                    &final_url,
                                                )
                                                .unwrap_or(
                                                    crate::audio::SYSTEM_DEFAULT_AUDIO_DEVICE,
                                                )
                                                .to_string();
                                                state.is_url_dialog_open = false;
                                                window.set_ime_allowed(false);
                                                state.url_input_text.clear();
                                                state.set_mic_input_mode(
                                                    true,
                                                    Some(dev_name.clone()),
                                                );
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
                                                let lower_url = final_url.to_ascii_lowercase();
                                                if !lower_url.starts_with("http://")
                                                    && !lower_url.starts_with("https://")
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
                                        EngineAction::Quit => {
                                            should_quit = true;
                                        }
                                        EngineAction::None => {}
                                    }
                                }
                                if should_quit {
                                    elwt.exit();
                                    return;
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
                                    apply_picked_file_paths(
                                        &mut state,
                                        paths,
                                        &mut initial_dir,
                                        window,
                                    );
                                }

                                // Poll IPC incoming paths (from secondary instances)
                                if let Ok(raw_paths) = ipc_rx.try_recv() {
                                    apply_ipc_file_paths(&mut state, raw_paths, window);
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
                                egui_gamepad_events.push(egui::Event::Key {
                                    key: egui::Key::ArrowDown,
                                    physical_key: None,
                                    pressed: true,
                                    repeat: false,
                                    modifiers: egui::Modifiers::NONE,
                                });
                                egui_gamepad_events.push(egui::Event::Key {
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
                            redraw_requested = true;
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
                        redraw_requested = true;
                    }

                    // Poll native file picker results immediately to eliminate UI latency
                    if let Ok(paths) = rfd_rx.try_recv() {
                        rfd_pending = false;
                        let mut state = app_state.lock().unwrap();
                        apply_picked_file_paths(&mut state, paths, &mut initial_dir, window);
                    }

                    // Poll IPC / macOS Finder openFiles results immediately
                    if let Ok(raw_paths) = ipc_rx.try_recv() {
                        let mut state = app_state.lock().unwrap();
                        apply_ipc_file_paths(&mut state, raw_paths, window);
                    }

                    let is_dialog_open =
                        *file_dialog.state() == egui_file_dialog::DialogState::Open || {
                            let state = app_state.lock().unwrap();
                            state.is_url_dialog_open
                        };

                    if let Some(ref mut gilrs) = gilrs {
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

                        let gamepad_input_active = is_game_mode || window.has_focus();
                        if !gamepad_input_active {
                            // Unfocused: drain and discard events so other apps own the controller.
                            while gilrs.next_event().is_some() {}
                            frame_left_stick_y = 0.0;
                            frame_right_stick_y = 0.0;
                            frame_left_trigger = 0.0;
                            frame_right_trigger = 0.0;
                        }
                        while let Some(gilrs::Event { event: g_event, .. }) = gilrs.next_event() {
                            match g_event {
                                gilrs::EventType::AxisChanged(axis, val, _) => match axis {
                                    gilrs::Axis::LeftStickY
                                        if val.abs() > frame_left_stick_y.abs() =>
                                    {
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
                            let is_pressed =
                                matches!(g_event, gilrs::EventType::ButtonPressed(_, _));
                            let is_repeated =
                                matches!(g_event, gilrs::EventType::ButtonRepeated(_, _));
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
                                            sync_keep_awake(
                                                &mut keep_awake,
                                                &mut keep_awake_mode,
                                                true,
                                                true,
                                            );
                                        } else {
                                            window.set_fullscreen(None);
                                            constrain_window_to_monitor(
                                                window,
                                                Some(desired_logical_w),
                                                Some(desired_logical_h),
                                                true,
                                                force_unit_scale,
                                            );
                                            sync_keep_awake(
                                                &mut keep_awake,
                                                &mut keep_awake_mode,
                                                false,
                                                false,
                                            );
                                        }
                                        continue;
                                    }
                                    _ => {}
                                }

                                if is_dialog_open {
                                    let is_url_open = app_state.lock().unwrap().is_url_dialog_open;
                                    let mut push_key = |key: egui::Key| {
                                        egui_gamepad_events.push(egui::Event::Key {
                                            key,
                                            physical_key: None,
                                            pressed: true,
                                            repeat: false,
                                            modifiers: egui::Modifiers::NONE,
                                        });
                                        egui_gamepad_events.push(egui::Event::Key {
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
                                                egui_gamepad_events.push(egui::Event::Key {
                                                    key: egui::Key::Tab,
                                                    physical_key: None,
                                                    pressed: true,
                                                    repeat: false,
                                                    modifiers: egui::Modifiers::SHIFT,
                                                });
                                                egui_gamepad_events.push(egui::Event::Key {
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
                                                egui_gamepad_events.push(egui::Event::Key {
                                                    key: egui::Key::Tab,
                                                    physical_key: None,
                                                    pressed: true,
                                                    repeat: false,
                                                    modifiers: egui::Modifiers::NONE,
                                                });
                                                egui_gamepad_events.push(egui::Event::Key {
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
                                                egui_gamepad_events.push(egui::Event::Key {
                                                    key: egui::Key::ArrowUp,
                                                    physical_key: None,
                                                    pressed: true,
                                                    repeat: false,
                                                    modifiers: egui::Modifiers::ALT,
                                                });
                                                egui_gamepad_events.push(egui::Event::Key {
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
                                            egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers: egui::Modifiers::NONE,
                                            });
                                            egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: false,
                                                repeat: false,
                                                modifiers: egui::Modifiers::NONE,
                                            });
                                            continue;
                                        }
                                        gilrs::Button::LeftTrigger => {
                                            egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers: egui::Modifiers::SHIFT,
                                            });
                                            egui_gamepad_events.push(egui::Event::Key {
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
                                        let mut push_tab = |shift: bool| {
                                            let modifiers = if shift {
                                                egui::Modifiers::SHIFT
                                            } else {
                                                egui::Modifiers::NONE
                                            };
                                            egui_gamepad_events.push(egui::Event::Key {
                                                key: egui::Key::Tab,
                                                physical_key: None,
                                                pressed: true,
                                                repeat: false,
                                                modifiers,
                                            });
                                            egui_gamepad_events.push(egui::Event::Key {
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
                                                    app_state.lock().unwrap().open_file_request =
                                                        true;
                                                } else {
                                                    egui_gamepad_events.push(egui::Event::Key {
                                                        key: egui::Key::Enter,
                                                        physical_key: None,
                                                        pressed: true,
                                                        repeat: false,
                                                        modifiers: egui::Modifiers::NONE,
                                                    });
                                                    egui_gamepad_events.push(egui::Event::Key {
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
                    } else if is_game_mode {
                        app_state.lock().unwrap().has_gamepad = true;
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

                    let is_minimized = window.is_minimized() == Some(true);
                    if !is_minimized && !redraw_requested {
                        window.request_redraw();
                        redraw_requested = true;
                    }
                    if is_uncapped && !is_minimized {
                        elwt.set_control_flow(ControlFlow::Poll);
                    } else {
                        elwt.set_control_flow(ControlFlow::WaitUntil(
                            Instant::now() + Duration::from_millis(16),
                        ));
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
