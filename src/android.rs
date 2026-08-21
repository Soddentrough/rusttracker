use std::sync::{Arc, Mutex, OnceLock};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;
use jni::objects::{JClass, JString};
use jni::JNIEnv;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    platform::android::activity::AndroidApp,
    platform::android::EventLoopBuilderExtAndroid,
    window::{Window, WindowId},
};

use crate::state::{AppState, VISUALIZERS};
use crate::engine::{EngineAction, VulkanEngine};
use crate::touch::{TouchGesture, TouchGestureController};

static FILE_PICKER_CHANNEL: OnceLock<(Sender<String>, Mutex<Receiver<String>>)> = OnceLock::new();

fn get_file_channel() -> &'static (Sender<String>, Mutex<Receiver<String>>) {
    FILE_PICKER_CHANNEL.get_or_init(|| {
        let (tx, rx) = channel();
        (tx, Mutex::new(rx))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rusttracker_app_MainActivity_nativeOnFileSelected<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    file_path: JString<'local>,
) {
    if let Ok(path) = env.get_string(&file_path) {
        let path_str: String = path.into();
        eprintln!("[RustTracker] JNI: Selected file path: {}", path_str);
        let (tx, _) = get_file_channel();
        let _ = tx.send(path_str);
    }
}

pub fn trigger_android_file_picker(app: &AndroidApp) {
    eprintln!("[RustTracker] Requesting Android SAF file picker...");
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    if let Ok(vm) = vm {
        if let Ok(mut env) = vm.attach_current_thread() {
            let activity_obj = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr() as *mut _) };
            if let Err(e) = env.call_method(&activity_obj, "openFilePicker", "()V", &[]) {
                eprintln!("[RustTracker] Failed to call openFilePicker on activity: {:?}", e);
            } else {
                eprintln!("[RustTracker] Successfully invoked openFilePicker via JNI!");
            }
        } else {
            eprintln!("[RustTracker] Failed to attach current thread to JavaVM");
        }
    }
}

struct AndroidRustTrackerApp {
    android_app: AndroidApp,
    app_state: Arc<Mutex<AppState>>,
    window: Option<Arc<Window>>,
    engine: Option<VulkanEngine>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    touch_controller: TouchGestureController,
    active_stream: Option<crate::audio::PlaybackHandle>,
    last_frame_time: Instant,
    is_suspended: bool,
    frame_count: u64,
}

fn create_egui_context() -> egui::Context {
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
    fonts.families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push("kenney_icons".to_owned());
    egui_ctx.set_fonts(fonts);
    egui_ctx
}

impl ApplicationHandler for AndroidRustTrackerApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        eprintln!("[RustTracker] Android Lifecycle: Resumed");
        self.is_suspended = false;
        self.last_frame_time = Instant::now();

        if self.window.is_none() {
            let win_attrs = Window::default_attributes()
                .with_title("RustTracker");
            match event_loop.create_window(win_attrs) {
                Ok(win) => {
                    let win = Arc::new(win);
                    eprintln!("[RustTracker] Created Android Native Window: {:?}", win.inner_size());
                    
                    let eng = pollster::block_on(VulkanEngine::new(win.clone()));
                    let egui_ctx = create_egui_context();
                    let state = egui_winit::State::new(
                        egui_ctx.clone(),
                        egui::ViewportId::ROOT,
                        &win,
                        Some(win.scale_factor() as f32),
                        None,
                        None,
                    );

                    self.egui_ctx = egui_ctx;
                    self.window = Some(win);
                    self.engine = Some(eng);
                    self.egui_state = Some(state);
                }
                Err(err) => {
                    eprintln!("[RustTracker] Failed to create window on Android: {:?}", err);
                }
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        eprintln!("[RustTracker] Android Lifecycle: Suspended");
        self.is_suspended = true;
        self.engine = None;
        self.window = None;
        self.egui_state = None;
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let Some(win) = self.window.clone() else {
            return;
        };
        if window_id != win.id() {
            return;
        }

        if let Some(ref mut st) = self.egui_state {
            let _ = st.on_window_event(&win, &event);
        }

        match event {
            WindowEvent::Resized(physical_size) => {
                eprintln!("[RustTracker] Android Window Resized: {:?}", physical_size);
                if physical_size.width > 0 && physical_size.height > 0 {
                    if let Some(ref mut eng) = self.engine {
                        eng.resize(physical_size);
                    }
                }
            }
            WindowEvent::Touch(touch) => {
                let size = win.inner_size();
                let is_portrait = size.width < size.height;
                let split_ratio = self.app_state.lock().unwrap().panel_split_ratio;
                let split_y = size.height as f32 * split_ratio;
                let has_video = self.app_state.lock().unwrap().has_video_stream || self.engine.as_ref().map(|e| e.has_video_stream()).unwrap_or(false);
                if let Some(gesture) = self.touch_controller.handle_touch(&touch, size.width as f32, size.height as f32, is_portrait, split_y) {
                    eprintln!("[RustTracker Touch] Detected gesture: {:?}", gesture);
                    if let Some(EngineAction::OpenFile) = Self::handle_gesture_static(&self.app_state, gesture, has_video, size.width as f32, size.height as f32) {
                        trigger_android_file_picker(&self.android_app);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if !self.is_suspended {
                    let dt = self.last_frame_time.elapsed().as_secs_f32().clamp(0.001, 0.1);
                    self.last_frame_time = Instant::now();

                    let size = win.inner_size();
                    let has_video = self.app_state.lock().unwrap().has_video_stream || self.engine.as_ref().map(|e| e.has_video_stream()).unwrap_or(false);

                    // Check for single tap timeout
                    if let Some(gesture) = self.touch_controller.update_pending_tap() {
                        if let Some(EngineAction::OpenFile) = Self::handle_gesture_static(&self.app_state, gesture, has_video, size.width as f32, size.height as f32) {
                            trigger_android_file_picker(&self.android_app);
                        }
                    }

                    // Check for SAF file picker result
                    let (_, rx_lock) = get_file_channel();
                    if let Ok(rx) = rx_lock.try_lock() {
                        while let Ok(path) = rx.try_recv() {
                            eprintln!("[RustTracker] Starting playback of SAF selected file: {}", path);
                            self.load_and_play_file(&path);
                        }
                    }

                    // Check for track ended / playlist auto-advance / pending load requests
                    let pending_load = {
                        let mut state = self.app_state.lock().unwrap();
                        if state.track_ended {
                            state.track_ended = false;
                            if state.playlist_index + 1 < state.playlist.len() {
                                state.playlist_index += 1;
                                state.load_request = Some(state.playlist[state.playlist_index].clone());
                            }
                        }
                        state.load_request.take()
                    };
                    if let Some(load_path) = pending_load {
                        self.load_and_play_file(&load_path);
                    }

                    let (Some(ref mut eng), Some(ref mut st)) = (self.engine.as_mut(), self.egui_state.as_mut()) else {
                        return;
                    };

                    self.frame_count += 1;
                    let fps = 1.0 / dt;
                    {
                        let mut state = self.app_state.lock().unwrap();
                        state.current_fps = if state.current_fps == 0.0 { fps } else { state.current_fps * 0.9 + fps * 0.1 };
                    }

                    // Per-frame state update, decay & smoothing (mirrors desktop pipeline)
                    let state_copy = {
                        let mut state = self.app_state.lock().unwrap();
                        let time_scale = (dt * 60.0).clamp(0.2, 5.0);

                        // VU decay
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

                        // Spectrum & Fire Heat decay
                        if state.spectrum_data.len() != state.raw_spectrum_data.len() {
                            state.spectrum_data = vec![0.0; state.raw_spectrum_data.len()];
                        }
                        for i in 0..state.raw_spectrum_data.len() {
                            if state.raw_spectrum_data[i] > state.spectrum_data[i] {
                                state.spectrum_data[i] = state.raw_spectrum_data[i];
                            } else {
                                state.spectrum_data[i] = (state.spectrum_data[i] - (1.5 * time_scale)).max(state.raw_spectrum_data[i]);
                            }
                            
                            if state.raw_spectrum_data[i] > state.fire_heat[i] {
                                state.fire_heat[i] = state.raw_spectrum_data[i];
                            } else {
                                state.fire_heat[i] = (state.fire_heat[i] - (1.5 * time_scale)).max(0.0);
                            }
                        }

                        // Spectrum History
                        if state.spectrum_history.len() > 120 {
                            state.spectrum_history.pop_front();
                        }
                        let cloned_spectrum = state.spectrum_data.clone();
                        state.spectrum_history.push_back(cloned_spectrum);

                        // Waveform temporal smoothing
                        let raw_wave = state.raw_waveform.clone();
                        if let Some(newest) = state.waveform_history.back_mut() {
                            let lerp_speed = (12.0 * dt).min(1.0);
                            for i in 0..newest.len().min(1024) {
                                newest[i] += (raw_wave[i] - newest[i]) * lerp_speed;
                            }
                        }

                        // OSD timer
                        if state.osd_timer > 0.0 {
                            state.osd_timer = (state.osd_timer - dt).max(0.0);
                            if state.osd_timer == 0.0 {
                                state.osd_text = None;
                            }
                        }

                        // Upload GPU uniforms
                        eng.update(&state, dt);

                        state.render_snapshot()
                    };

                    let mut file_dialog = egui_file_dialog::FileDialog::new();

                    match eng.render(
                        &win,
                        &self.egui_ctx,
                        st,
                        &state_copy,
                        &mut file_dialog,
                        Vec::new(),
                    ) {
                        Ok((action, ui_time, render_time, fire_time, vis_shader_time, fft_time, _smoke_time, _ferro_time, _biolum_time, _resynth_time)) => {
                            {
                                let mut state = self.app_state.lock().unwrap();
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

                                if self.frame_count % 60 == 0 {
                                    let vis_name = crate::state::VISUALIZERS.get(state.current_visualizer_idx).map(|v| v.name).unwrap_or("Unknown");
                                    eprintln!("[RustTracker Vis Perf] Vis {} ({}): FPS={:.1} | Shader={:.1}us | Render={:.1}us | UI={:.1}us",
                                        state.visualizer_mode, vis_name, state.current_fps, state.stats.shader_us, state.stats.render_us, state.stats.ui_us);
                                }
                            }

                            match action {
                                EngineAction::OpenFile => {
                                    trigger_android_file_picker(&self.android_app);
                                }
                                EngineAction::LoadFiles(files, append) => {
                                    if !files.is_empty() {
                                        if append {
                                            let mut state = self.app_state.lock().unwrap();
                                            state.playlist.extend(files);
                                        } else {
                                            self.load_and_play_file(&files[0]);
                                            let mut state = self.app_state.lock().unwrap();
                                            state.playlist = files;
                                            state.playlist_index = 0;
                                        }
                                    }
                                }
                                EngineAction::SetMobileHudTab(tab) => {
                                    self.app_state.lock().unwrap().mobile_hud_tab = tab;
                                }
                                EngineAction::Seek(pct) => {
                                    let mut state = self.app_state.lock().unwrap();
                                    state.seek_request = Some(pct as f64 * state.duration_seconds);
                                }
                                EngineAction::SetForceStereo(force) => {
                                    self.app_state.lock().unwrap().force_stereo_downmix = force;
                                }
                                EngineAction::SetAudioTrack(track_idx) => {
                                    let mut state = self.app_state.lock().unwrap();
                                    state.audio_track_request = Some(track_idx);
                                }
                                _ => {}
                            }
                        }
                        Err(wgpu::SurfaceStatus::Lost | wgpu::SurfaceStatus::Outdated) => {
                            eprintln!("[RustTracker] Surface lost or outdated, resizing...");
                            let size = win.inner_size();
                            if size.width > 0 && size.height > 0 {
                                eng.resize(size);
                            }
                        }
                        Err(err) => {
                            eprintln!("[RustTracker] Android Render error: {:?}", err);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.is_suspended {
            if let Some(ref win) = self.window {
                win.request_redraw();
            }
        }
    }
}

impl AndroidRustTrackerApp {
    fn handle_gesture_static(
        app_state: &Arc<Mutex<AppState>>,
        gesture: TouchGesture,
        has_video: bool,
        window_width: f32,
        window_height: f32,
    ) -> Option<EngineAction> {
        let mut state = app_state.lock().unwrap();
        match gesture {
            // Top pane swipe in portrait: cycle HUD tabs (Channels -> Heatmap/Lyrics -> Info -> Video)
            TouchGesture::TopPaneSwipeLeft => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                state.mobile_hud_tab = state.mobile_hud_tab.next(has_video);
                state.osd_text = Some(match state.mobile_hud_tab {
                    crate::state::MobileHudTab::Channels => "Channels".to_string(),
                    crate::state::MobileHudTab::Heatmap => if state.lyrics.is_some() { "Lyrics".to_string() } else { "Heatmap".to_string() },
                    crate::state::MobileHudTab::Info => "Track Info".to_string(),
                    crate::state::MobileHudTab::Video => "Video Stream".to_string(),
                });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::TopPaneSwipeRight => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                state.mobile_hud_tab = state.mobile_hud_tab.prev(has_video);
                state.osd_text = Some(match state.mobile_hud_tab {
                    crate::state::MobileHudTab::Channels => "Channels".to_string(),
                    crate::state::MobileHudTab::Heatmap => if state.lyrics.is_some() { "Lyrics".to_string() } else { "Heatmap".to_string() },
                    crate::state::MobileHudTab::Info => "Track Info".to_string(),
                    crate::state::MobileHudTab::Video => "Video Stream".to_string(),
                });
                state.osd_timer = 1.5;
                None
            }
            // Top pane swipe up in portrait: If on video pane or video exists, toggle fullscreen video!
            TouchGesture::TopPaneSwipeUp => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                } else if has_video {
                    state.video_mode = 3;
                    state.mobile_hud_tab = crate::state::MobileHudTab::Video;
                }
                None
            }
            // Top pane swipe down in portrait: If in fullscreen video, return to split view
            TouchGesture::TopPaneSwipeDown => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                None
            }
            // Bottom pane swipe in portrait OR Horizontal swipe in landscape: cycle visualizers
            TouchGesture::BottomPaneSwipeLeft | TouchGesture::LandscapeSwipeLeft => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                let len = VISUALIZERS.len();
                let mut idx = state.current_visualizer_idx;
                for _ in 0..len {
                    idx = (idx + 1) % len;
                    if state.vis_enabled.get(idx).copied().unwrap_or(true) {
                        break;
                    }
                }
                state.current_visualizer_idx = idx;
                state.visualizer_mode = VISUALIZERS[idx].id;
                state.osd_text = Some(VISUALIZERS[idx].name.to_string());
                state.osd_timer = 2.0;
                None
            }
            TouchGesture::BottomPaneSwipeRight | TouchGesture::LandscapeSwipeRight => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                let len = VISUALIZERS.len();
                let mut idx = state.current_visualizer_idx;
                for _ in 0..len {
                    idx = if idx == 0 { len - 1 } else { idx - 1 };
                    if state.vis_enabled.get(idx).copied().unwrap_or(true) {
                        break;
                    }
                }
                state.current_visualizer_idx = idx;
                state.visualizer_mode = VISUALIZERS[idx].id;
                state.osd_text = Some(VISUALIZERS[idx].name.to_string());
                state.osd_timer = 2.0;
                None
            }
            TouchGesture::BottomPaneSwipeUp => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                None
            }
            TouchGesture::BottomPaneSwipeDown => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                None
            }
            // Vertical swipe in landscape: rotate full screen HUD overlays
            TouchGesture::LandscapeSwipeUp => {
                state.mobile_hud_tab = state.mobile_hud_tab.next(has_video);
                state.show_hud = true;
                state.osd_text = Some(match state.mobile_hud_tab {
                    crate::state::MobileHudTab::Channels => "Channels".to_string(),
                    crate::state::MobileHudTab::Heatmap => if state.lyrics.is_some() { "Lyrics".to_string() } else { "Heatmap".to_string() },
                    crate::state::MobileHudTab::Info => "Track Info".to_string(),
                    crate::state::MobileHudTab::Video => "Video Stream".to_string(),
                });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::LandscapeSwipeDown => {
                state.mobile_hud_tab = state.mobile_hud_tab.prev(has_video);
                state.show_hud = true;
                state.osd_text = Some(match state.mobile_hud_tab {
                    crate::state::MobileHudTab::Channels => "Channels".to_string(),
                    crate::state::MobileHudTab::Heatmap => if state.lyrics.is_some() { "Lyrics".to_string() } else { "Heatmap".to_string() },
                    crate::state::MobileHudTab::Info => "Track Info".to_string(),
                    crate::state::MobileHudTab::Video => "Video Stream".to_string(),
                });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::SingleTap { x, y } => {
                let is_portrait = window_width < window_height;

                // Check if top right [📂 OPEN] header button was tapped
                if is_portrait && y < 160.0 && x > window_width * 0.72 {
                    return Some(EngineAction::OpenFile);
                }

                // If no file loaded, keep splash screen and HUD active
                if !state.file_loaded {
                    return None;
                }

                // Single tap always toggles Play / Pause
                state.is_paused = !state.is_paused;
                state.osd_text = Some(if state.is_paused { "Paused".to_string() } else { "Playing".to_string() });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::TwoFingerDoubleTap | TouchGesture::TwoFingerTap => {
                if state.audio_tracks.len() > 1 {
                    let next_track = (state.selected_audio_track + 1) % state.audio_tracks.len();
                    state.audio_track_request = Some(next_track);
                    let track_title = state.audio_tracks.get(next_track).map(|t| t.title.clone()).unwrap_or_default();
                    state.osd_text = Some(track_title);
                    state.osd_timer = 2.0;
                } else {
                    state.osd_text = Some("1 Audio Track Present".to_string());
                    state.osd_timer = 1.5;
                }
                None
            }
            TouchGesture::DoubleTap { .. } => {
                // Double tap toggles HUD visibility
                state.show_hud = !state.show_hud;
                state.osd_text = Some(if state.show_hud { "HUD: Visible".to_string() } else { "HUD: Hidden".to_string() });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::Scrub { pct } => {
                state.seek_request = Some(pct as f64 * state.duration_seconds);
                None
            }
            TouchGesture::ScrubEnd => None,
        }
    }

    fn load_and_play_file(&mut self, path: &str) {
        log_android(3, &format!("Loading and playing file: {}", path));
        self.active_stream = None;
        {
            let mut state = self.app_state.lock().unwrap();
            state.has_video_stream = false;
            state.video_frame_rx = None;
            state.free_video_frame_tx = None;
            state.video_info = None;
            state.audio_tracks.clear();
            state.selected_audio_track = 0;
            state.lyrics = None;
        }
        if let Some(engine) = self.engine.as_mut() {
            engine.clear_video_state();
        }

        match crate::audio::start_audio_thread(path, false, Arc::clone(&self.app_state)) {
            Ok(stream) => {
                log_android(3, &format!("Successfully started audio thread for {}", path));
                eprintln!("[RustTracker] Successfully started audio thread for {}", path);
                let mut state = self.app_state.lock().unwrap();
                state.file_loaded = true;
                state.is_paused = false;
                state.track_ended = false;
                state.song_title = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();
                state.playlist = vec![path.to_string()];
                state.playlist_index = 0;
                state.lyrics = crate::lyrics::load_lyrics_for_file(path).map(std::sync::Arc::new);
                if let Some(l) = &state.lyrics {
                    log_android(3, &format!("Loaded {} lines from {:?}", l.lines.len(), l.file_name));
                    eprintln!("[RustTracker Lyrics] Loaded {} lines from {:?}", l.lines.len(), l.file_name);
                }
                self.active_stream = Some(stream);
            }
            Err(e) => {
                log_android(6, &format!("Failed to start audio thread: {:?}", e));
                eprintln!("[RustTracker] Failed to start audio thread: {:?}", e);
                let mut state = self.app_state.lock().unwrap();
                state.osd_text = Some("Load Failed".to_string());
                state.osd_timer = 3.0;
            }
        }
    }
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
}

pub fn log_android(prio: i32, msg: &str) {
    if let Ok(c_msg) = std::ffi::CString::new(msg) {
        unsafe {
            __android_log_write(prio, c"RustTracker".as_ptr(), c_msg.as_ptr());
        }
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    std::panic::set_hook(Box::new(|panic_info| {
        let msg = format!("[PANIC] {}", panic_info);
        log_android(6, &msg);
        eprintln!("[RustTracker PANIC] {}", panic_info);
    }));

    log_android(3, "Starting RustTracker on Android with Touch Gestures, SAF, and Audio Engine...");
    eprintln!("[RustTracker] Starting on Android with Touch Gestures, SAF, and Audio Engine...");

    let event_loop = EventLoop::builder()
        .with_android_app(app.clone())
        .build()
        .expect("Failed to create Android EventLoop");

    let app_state = Arc::new(Mutex::new(AppState::new("RustTracker Mobile".to_string())));
    let egui_ctx = create_egui_context();

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut android_app = AndroidRustTrackerApp {
        android_app: app,
        app_state,
        window: None,
        engine: None,
        egui_ctx,
        egui_state: None,
        touch_controller: TouchGestureController::new(),
        active_stream: None,
        last_frame_time: Instant::now(),
        is_suspended: false,
        frame_count: 0,
    };

    let _ = event_loop.run_app(&mut android_app);
}
