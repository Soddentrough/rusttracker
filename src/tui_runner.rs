use crate::audio;
use crate::cli::reconcile_passthrough_change_osd;
use crate::gui_runner::{MAX_PANEL_SPLIT_RATIO, MIN_PANEL_SPLIT_RATIO};
use crate::state::AppState;
use crate::ui;
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

pub(crate) struct Tui {
    pub(crate) terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
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

pub(crate) fn adjust_tui_volume(state: &mut AppState, delta: f32) {
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
    if state.audio_track_volumes.iter().any(|&v| v > 0.0) {
        state.tui_pre_mute_volumes = None;
    }
    let display_idx = state
        .active_audio_tracks
        .iter()
        .copied()
        .find(|&idx| idx == sel)
        .or_else(|| state.active_audio_tracks.first().copied())
        .unwrap_or(0);
    let current_vol = state
        .audio_track_volumes
        .get(display_idx)
        .copied()
        .unwrap_or(1.0);
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
            if state.is_mic_input {
                state.set_audio_devices(Vec::new(), refreshed);
            } else {
                state.set_audio_devices(refreshed, Vec::new());
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
        KeyCode::Char('g') | KeyCode::Char('G') => {
            state.toggle_mpegh_bitstream();
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            let sel = state.selected_audio_track;
            if state.active_audio_tracks.is_empty() {
                state.active_audio_tracks.push(sel);
            }
            let num_tracks = state.audio_tracks.len().max(sel + 1);
            if state.audio_track_volumes.len() != num_tracks {
                state.audio_track_volumes = vec![1.0; num_tracks];
            }
            let is_currently_zero = state.audio_track_volumes.iter().all(|&v| v == 0.0);
            if is_currently_zero {
                if let Some(saved) = state.tui_pre_mute_volumes.take()
                    && saved.iter().any(|&v| v > 0.0)
                {
                    for (idx, vol) in state.audio_track_volumes.iter_mut().enumerate() {
                        *vol = saved.get(idx).copied().unwrap_or(1.0).clamp(0.0, 1.0);
                    }
                } else {
                    state.audio_track_volumes.fill(1.0);
                }
            } else {
                state.tui_pre_mute_volumes = Some(state.audio_track_volumes.clone());
                state.audio_track_volumes.fill(0.0);
            }
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
            state.osd_text = Some(if is_currently_zero {
                "Unmuted".to_string()
            } else {
                "Muted".to_string()
            });
            state.osd_timer = 2.0;
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
                let _ = state.rt_control.take_track_ended();
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
                let _ = state.rt_control.take_track_ended();
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
                let base_top_h = ((meta_len as u16).saturating_add(2).max(12)).max(14) as f32;
                let rel_y = row.saturating_sub(term_area.y + 1) as f32;
                let new_ratio = 0.5 + (rel_y - base_top_h) / content_h;
                state.panel_split_ratio =
                    new_ratio.clamp(MIN_PANEL_SPLIT_RATIO, MAX_PANEL_SPLIT_RATIO);
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

pub(crate) fn run_tui<B: Backend>(
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

        let mut redraw_immediately = false;
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
                    CEvent::Resize(_, _) => {
                        redraw_immediately = true;
                        break;
                    }
                    _ => {}
                }
                if !crossterm::event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if redraw_immediately || last_tick.elapsed() >= tick_rate {
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
