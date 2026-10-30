use jni::JNIEnv;
use jni::objects::{JClass, JString};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    platform::android::EventLoopBuilderExtAndroid,
    platform::android::activity::AndroidApp,
    window::{Window, WindowId},
};

use crate::engine::{EngineAction, VulkanEngine};
use crate::state::{AppState, VISUALIZERS};
use crate::touch::{TouchGesture, TouchGestureController};

#[derive(Debug, Clone)]
pub enum FilePickerEvent {
    LoadingStarted(String),
    Selected(String),
}

static FILE_PICKER_CHANNEL: OnceLock<(Sender<FilePickerEvent>, Mutex<Receiver<FilePickerEvent>>)> =
    OnceLock::new();
static GLOBAL_APP_STATE: OnceLock<Arc<Mutex<AppState>>> = OnceLock::new();

#[derive(Debug)]
pub enum AppCustomEvent {
    Destroy,
    Wake,
}

static EVENT_LOOP_PROXY: OnceLock<
    Mutex<Option<winit::event_loop::EventLoopProxy<AppCustomEvent>>>,
> = OnceLock::new();

fn get_file_channel() -> &'static (Sender<FilePickerEvent>, Mutex<Receiver<FilePickerEvent>>) {
    FILE_PICKER_CHANNEL.get_or_init(|| {
        let (tx, rx) = channel();
        (tx, Mutex::new(rx))
    })
}

fn wake_event_loop() {
    if let Some(lock) = EVENT_LOOP_PROXY.get()
        && let Ok(guard) = lock.lock()
        && let Some(proxy) = guard.as_ref()
    {
        let _ = proxy.send_event(AppCustomEvent::Wake);
    }
}

fn handle_back_navigation(app_state: &Arc<Mutex<AppState>>) -> bool {
    if let Ok(mut state) = app_state.lock() {
        if state.is_url_dialog_open {
            state.is_url_dialog_open = false;
            return true;
        }
        if state.show_vis_picker {
            state.show_vis_picker = false;
            return true;
        }
        if state.show_help {
            state.show_help = false;
            return true;
        }
        if state.show_stats {
            state.show_stats = false;
            return true;
        }
        if state.video_mode == 3 {
            state.video_mode = 0;
            return true;
        }
        if state.file_loaded && !state.show_hud {
            state.show_hud = true;
            return true;
        }
    }
    false
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rusttracker_app_MainActivity_nativeOnBackPressed<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jni::sys::jboolean {
    if let Some(app_state) = GLOBAL_APP_STATE.get()
        && handle_back_navigation(app_state)
    {
        return jni::sys::JNI_TRUE;
    }
    jni::sys::JNI_FALSE
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rusttracker_app_MainActivity_nativeOnDestroy<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    eprintln!("[RustTracker] JNI: nativeOnDestroy invoked from Android Activity");
    if let Some(lock) = EVENT_LOOP_PROXY.get()
        && let Ok(guard) = lock.lock()
        && let Some(proxy) = guard.as_ref()
    {
        let _ = proxy.send_event(AppCustomEvent::Destroy);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rusttracker_app_MainActivity_nativeOnFileLoadingStarted<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    display_name: JString<'local>,
) {
    if let Ok(name) = env.get_string(&display_name) {
        let name_str: String = name.into();
        eprintln!("[RustTracker] JNI: File loading started: {}", name_str);
        if let Some(app_state) = GLOBAL_APP_STATE.get()
            && let Ok(mut state) = app_state.lock()
        {
            state.is_paused = true;
            state.song_title = format!("Loading {}...", name_str);
            state.osd_text = Some(format!("Loading {}...", name_str));
            state.osd_timer = 15.0;
        }
        let (tx, _) = get_file_channel();
        let _ = tx.send(FilePickerEvent::LoadingStarted(name_str));
        wake_event_loop();
    }
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
        let _ = tx.send(FilePickerEvent::Selected(path_str));
        wake_event_loop();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rusttracker_app_MainActivity_nativeOnFileStatus<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    status_msg: JString<'local>,
) {
    if let Ok(msg) = env.get_string(&status_msg) {
        let msg_str: String = msg.into();
        eprintln!("[RustTracker] JNI: File status: {}", msg_str);
        if let Some(app_state) = GLOBAL_APP_STATE.get()
            && let Ok(mut state) = app_state.lock()
        {
            state.osd_text = Some(msg_str);
            state.osd_timer = 8.0;
        }
        wake_event_loop();
    }
}

pub fn trigger_android_file_picker(app: &AndroidApp) {
    eprintln!("[RustTracker] Requesting Android SAF file picker...");
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    if let Ok(vm) = vm {
        if let Ok(mut env) = vm.attach_current_thread() {
            let activity_obj =
                unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr() as *mut _) };
            if let Err(e) = env.call_method(&activity_obj, "openFilePicker", "()V", &[]) {
                eprintln!(
                    "[RustTracker] Failed to call openFilePicker on activity: {:?}",
                    e
                );
            } else {
                eprintln!("[RustTracker] Successfully invoked openFilePicker via JNI!");
            }
        } else {
            eprintln!("[RustTracker] Failed to attach current thread to JavaVM");
        }
    }
}

pub fn query_android_display_refresh_rate(app: &AndroidApp) -> f32 {
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut _) };
    if let Ok(vm) = vm
        && let Ok(mut env) = vm.attach_current_thread()
    {
        let activity_obj =
            unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr() as *mut _) };
        if let Ok(val) = env.call_method(&activity_obj, "getDisplayRefreshRate", "()F", &[])
            && let Ok(hz) = val.f()
            && (30.0..=240.0).contains(&hz)
        {
            return hz;
        }
    }
    60.0
}

struct AndroidRustTrackerApp {
    android_app: AndroidApp,
    app_state: Arc<Mutex<AppState>>,
    window: Option<Arc<Window>>,
    engine: Option<VulkanEngine>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    file_dialog: egui_file_dialog::FileDialog,
    touch_controller: TouchGestureController,
    active_stream: Option<crate::audio::PlaybackHandle>,
    last_frame_time: Instant,
    frame_dt_ring: [f32; 60],
    frame_dt_idx: usize,
    display_refresh_hz: f32,
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
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push("kenney_icons".to_owned());
    egui_ctx.set_fonts(fonts);
    egui_ctx
}

impl ApplicationHandler<AppCustomEvent> for AndroidRustTrackerApp {
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppCustomEvent) {
        match event {
            AppCustomEvent::Destroy => {
                eprintln!(
                    "[RustTracker] Android Lifecycle: Received Destroy event, exiting event loop..."
                );
                event_loop.exit();
            }
            AppCustomEvent::Wake => {
                if !self.is_suspended
                    && let Some(ref win) = self.window
                {
                    win.request_redraw();
                }
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        eprintln!("[RustTracker] Android Lifecycle: Exiting event loop, cleaning up resources...");
        if let Some(ref mut eng) = self.engine {
            eng.suspend_surface();
        }
        self.window = None;
        self.engine = None;
        self.egui_state = None;
        self.active_stream = None;
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        eprintln!("[RustTracker] Android Lifecycle: Resumed");
        self.is_suspended = false;
        self.touch_controller.reset();
        self.last_frame_time = Instant::now();
        self.display_refresh_hz = query_android_display_refresh_rate(&self.android_app);
        let vsync_dt = 1.0 / self.display_refresh_hz.max(30.0);
        self.frame_dt_ring = [vsync_dt; 60];
        self.frame_dt_idx = 0;
        self.frame_count = 0;
        eprintln!(
            "[RustTracker] Display VSync refresh rate: {:.2} Hz ({:.2} ms)",
            self.display_refresh_hz,
            vsync_dt * 1000.0
        );

        if self.window.is_none() {
            let win_attrs = Window::default_attributes().with_title("RustTracker");
            match event_loop.create_window(win_attrs) {
                Ok(win) => {
                    let win = Arc::new(win);
                    eprintln!(
                        "[RustTracker] Created Android Native Window: {:?}",
                        win.inner_size()
                    );

                    let egui_ctx = self.egui_ctx.clone();
                    let state = egui_winit::State::new(
                        egui_ctx,
                        egui::ViewportId::ROOT,
                        &win,
                        Some(win.scale_factor() as f32),
                        None,
                        None,
                    );

                    if let Some(ref mut eng) = self.engine {
                        eprintln!(
                            "[RustTracker] Re-attaching existing VulkanEngine to resumed window..."
                        );
                        eng.resume_surface(win.clone());
                    } else {
                        eprintln!("[RustTracker] Initializing VulkanEngine for the first time...");
                        let eng = pollster::block_on(VulkanEngine::new(win.clone()));
                        {
                            let mut app_state = self.app_state.lock().unwrap();
                            app_state.apply_gpu_capability_profile(
                                &eng.adapter_info,
                                eng.config.width,
                                eng.config.height,
                            );
                            eprintln!(
                                "[RustTracker] GPU Adapter: '{}' ({:?}) @ {}x{} -> Tier: {:?}",
                                eng.adapter_info.name,
                                eng.adapter_info.device_type,
                                eng.config.width,
                                eng.config.height,
                                app_state.gpu_capability_tier
                            );
                        }
                        self.engine = Some(eng);
                    }

                    win.request_redraw();
                    self.window = Some(win);
                    self.egui_state = Some(state);
                }
                Err(err) => {
                    eprintln!(
                        "[RustTracker] Failed to create window on Android: {:?}",
                        err
                    );
                }
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        eprintln!("[RustTracker] Android Lifecycle: Suspended");
        self.is_suspended = true;
        self.touch_controller.reset();
        if let Some(ref mut eng) = self.engine {
            eng.suspend_surface();
        }
        self.window = None;
        self.egui_state = None;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(win) = self.window.clone() else {
            return;
        };
        if window_id != win.id() {
            return;
        };

        if let Some(ref mut st) = self.egui_state {
            let _ = st.on_window_event(&win, &event);
        }

        match event {
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key:
                            Key::Named(NamedKey::BrowserBack) | Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                if handle_back_navigation(&self.app_state) {
                    win.set_ime_allowed(false);
                    win.request_redraw();
                } else {
                    eprintln!("[RustTracker] Back/Escape key pressed, exiting event loop...");
                    event_loop.exit();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Character(ref c),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } if c.eq_ignore_ascii_case("s") => {
                let mut state = self.app_state.lock().unwrap();
                if !state.is_url_dialog_open {
                    state.show_stats = !state.show_stats;
                    state.osd_text = Some(if state.show_stats {
                        "Engine Stats: Visible".to_string()
                    } else {
                        "Engine Stats: Hidden".to_string()
                    });
                    state.osd_timer = 1.5;
                }
            }
            WindowEvent::Resized(physical_size) => {
                eprintln!("[RustTracker] Android Window Resized: {:?}", physical_size);
                if physical_size.width > 0
                    && physical_size.height > 0
                    && let Some(ref mut eng) = self.engine
                {
                    eng.resize(physical_size);
                }
            }
            WindowEvent::Touch(touch) => {
                let size = win.inner_size();
                let scale_factor = (win.scale_factor() as f32).max(1.0);
                self.touch_controller.set_scale_factor(scale_factor);
                let is_portrait = size.width < size.height;
                let split_ratio = self.app_state.lock().unwrap().panel_split_ratio;
                let min_h_px = 220.0 * scale_factor;
                let split_y = (size.height as f32 * split_ratio)
                    .clamp(min_h_px, (size.height as f32 - min_h_px).max(min_h_px));
                let has_video = self.app_state.lock().unwrap().has_video_stream
                    || self
                        .engine
                        .as_ref()
                        .map(|e| e.has_video_stream())
                        .unwrap_or(false);
                if let Some(gesture) = self.touch_controller.handle_touch(
                    &touch,
                    size.width as f32,
                    size.height as f32,
                    is_portrait,
                    split_y,
                ) {
                    eprintln!("[RustTracker Touch] Detected gesture: {:?}", gesture);
                    if let Some(EngineAction::OpenFile) = Self::handle_gesture_static(
                        &self.app_state,
                        gesture,
                        has_video,
                        size.width as f32,
                        size.height as f32,
                        scale_factor,
                    ) {
                        self.touch_controller.reset();
                        trigger_android_file_picker(&self.android_app);
                    }
                }
            }
            WindowEvent::RedrawRequested if !self.is_suspended => {
                let now = Instant::now();
                let raw_dt = now.duration_since(self.last_frame_time).as_secs_f32();
                self.last_frame_time = now;
                let dt = raw_dt.clamp(0.001, 0.1);

                let size = win.inner_size();
                let scale_factor = (win.scale_factor() as f32).max(1.0);
                self.touch_controller.set_scale_factor(scale_factor);
                let has_video = self.app_state.lock().unwrap().has_video_stream
                    || self
                        .engine
                        .as_ref()
                        .map(|e| e.has_video_stream())
                        .unwrap_or(false);

                // Check for single tap timeout
                if let Some(gesture) = self.touch_controller.update_pending_tap() {
                    let is_track_info_open_btn = if let TouchGesture::SingleTap { x, y } = gesture {
                        let is_portrait = size.width < size.height;
                        let st = self.app_state.lock().unwrap();
                        let in_info_hdr = if is_portrait {
                            st.mobile_hud_tab == crate::state::MobileHudTab::Info
                                && x > (size.width as f32 * 0.65)
                        } else {
                            st.show_hud && st.video_mode == 0 && x > (size.width as f32 * 0.82)
                        };
                        in_info_hdr && y < (115.0 * scale_factor)
                    } else {
                        false
                    };
                    let wants_pointer = (self.egui_ctx.egui_wants_pointer_input()
                        || self.egui_ctx.is_pointer_over_egui())
                        && !is_track_info_open_btn;
                    if !wants_pointer
                        && let Some(EngineAction::OpenFile) = Self::handle_gesture_static(
                            &self.app_state,
                            gesture,
                            has_video,
                            size.width as f32,
                            size.height as f32,
                            scale_factor,
                        )
                    {
                        self.touch_controller.reset();
                        trigger_android_file_picker(&self.android_app);
                    }
                }

                // Check for SAF file picker events
                let (_, rx_lock) = get_file_channel();
                if let Ok(rx) = rx_lock.try_lock() {
                    let mut latest_selected_path = None;
                    while let Ok(evt) = rx.try_recv() {
                        match evt {
                            FilePickerEvent::LoadingStarted(display_name) => {
                                eprintln!(
                                    "[RustTracker] Stopping active stream while cloud/storage file loads: {}",
                                    display_name
                                );
                                self.active_stream = None;
                                latest_selected_path = None;
                                if let Some(engine) = self.engine.as_mut() {
                                    engine.clear_video_state();
                                }
                                let mut state = self.app_state.lock().unwrap();
                                state.is_paused = true;
                                state.current_seconds = 0.0;
                                state.duration_seconds = 0.0;
                                state.song_title = format!("Loading {}...", display_name);
                                state.raw_channel_vus.fill(0.0);
                                state.channel_vus.fill(0.0);
                                state.peak_vus.fill(0.0);
                                state.raw_spectrum_data.fill(0.0);
                                state.spectrum_data.fill(0.0);
                                state.fire_heat.fill(0.0);
                            }
                            FilePickerEvent::Selected(path) => {
                                latest_selected_path = Some(path);
                            }
                        }
                    }
                    if let Some(path) = latest_selected_path {
                        eprintln!(
                            "[RustTracker] Starting playback of SAF selected file: {}",
                            path
                        );
                        self.load_and_play_file(&path);
                    }
                }

                // Check for audio device reconnect / track ended / playlist auto-advance / pending load requests
                let (pending_load, resume_pos_on_reload) = {
                    let mut state = self.app_state.lock().unwrap();
                    let mut resume_pos_on_reload: Option<f64> = None;
                    if state.audio_device_lost {
                        state.audio_device_lost = false;
                        if state.playlist_index < state.playlist.len() {
                            let reload = state.playlist[state.playlist_index].clone();
                            if !reload.is_empty() {
                                state.load_request = Some(reload);
                                state.surround_upmix.user_overridden = false;
                                resume_pos_on_reload = Some(state.current_seconds);
                                state.osd_text = Some("Audio Output Reconnected".to_string());
                                state.osd_timer = 3.0;
                            }
                        }
                    }
                    if let Some(osd_msg) = state.audio_topology_reload_request.take()
                        && state.playlist_index < state.playlist.len()
                    {
                        let reload = state.playlist[state.playlist_index].clone();
                        if !reload.is_empty() && state.file_loaded {
                            state.load_request = Some(reload);
                            state.surround_upmix.user_overridden = false;
                            resume_pos_on_reload = Some(state.current_seconds);
                            state.osd_text = Some(osd_msg);
                            state.osd_timer = 3.0;
                        }
                    }
                    if let Some(new_dev) = state.audio_device_change_request.take()
                        && state.playlist_index < state.playlist.len()
                    {
                        let reload = state.playlist[state.playlist_index].clone();
                        if !reload.is_empty() {
                            state.load_request = Some(reload);
                            state.surround_upmix.user_overridden = false;
                            resume_pos_on_reload = Some(state.current_seconds);
                            state.osd_text = Some(format!("Audio Output: {}", new_dev));
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
                    if resume_pos_on_reload.is_none() && state.load_request.is_some() {
                        state.surround_upmix.user_overridden = false;
                    }
                    (state.load_request.take(), resume_pos_on_reload)
                };
                if let Some(load_path) = pending_load {
                    self.load_and_play_file(&load_path);
                    if let Some(pos) = resume_pos_on_reload
                        && pos > 0.1
                    {
                        let mut state = self.app_state.lock().unwrap();
                        state.current_seconds = pos;
                        state.seek_request = Some(pos);
                        state.seek_epoch += 1;
                    }
                }

                let (Some(ref mut eng), Some(ref mut st)) =
                    (self.engine.as_mut(), self.egui_state.as_mut())
                else {
                    return;
                };

                self.frame_count += 1;
                if self.frame_count > 2 && raw_dt > 0.0 && raw_dt < 0.25 {
                    self.frame_dt_ring[self.frame_dt_idx] = raw_dt;
                    self.frame_dt_idx = (self.frame_dt_idx + 1) % self.frame_dt_ring.len();
                }
                let min_vsync_dt = 1.0 / self.display_refresh_hz.max(30.0);
                let avg_dt = (self.frame_dt_ring.iter().sum::<f32>()
                    / (self.frame_dt_ring.len() as f32))
                    .max(min_vsync_dt);
                let fps = 1.0 / avg_dt;

                // Per-frame state update, decay & smoothing (mirrors desktop pipeline)
                let phase_timer = Instant::now();
                let (state_copy, phase_lock_update_us, phase_snapshot_us) = {
                    let mut state = self.app_state.lock().unwrap();
                    state.current_fps = fps;
                    state.update_visualizer_perf_guard(dt, self.display_refresh_hz);
                    let time_scale = (dt * 60.0).clamp(0.2, 5.0);

                    if state.file_loaded {
                        // VU decay
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
                                state.spectrum_data[i] = (state.spectrum_data[i]
                                    - (1.5 * time_scale))
                                    .max(state.raw_spectrum_data[i]);
                            }

                            if state.raw_spectrum_data[i] > state.fire_heat[i] {
                                state.fire_heat[i] = state.raw_spectrum_data[i];
                            } else {
                                state.fire_heat[i] =
                                    (state.fire_heat[i] - (1.5 * time_scale)).max(0.0);
                            }
                        }

                        // Spectrum History (reuse front allocation instead of cloning a new Vec every frame)
                        let mut recycled = if state.spectrum_history.len() > 120 {
                            state.spectrum_history.pop_front().unwrap_or_default()
                        } else {
                            Vec::with_capacity(state.spectrum_data.len())
                        };
                        recycled.clear();
                        recycled.extend_from_slice(&state.spectrum_data);
                        state.spectrum_history.push_back(recycled);
                    }

                    // OSD timer
                    state.tick_osd_timer(dt);

                    // Upload GPU uniforms
                    eng.update(&state, dt);
                    let lock_update_us = phase_timer.elapsed().as_micros() as f32;

                    let snap_timer = Instant::now();
                    let snap = state.render_snapshot();
                    let snap_us = snap_timer.elapsed().as_micros() as f32;
                    (snap, lock_update_us, snap_us)
                };

                match eng.render(
                    &win,
                    &self.egui_ctx,
                    st,
                    &state_copy,
                    &mut self.file_dialog,
                    Vec::new(),
                ) {
                    Ok((
                        action,
                        ui_time,
                        render_time,
                        fire_time,
                        fft_time,
                        vis_shader_time,
                        phase_surface_us,
                        phase_egui_us,
                        phase_encode_us,
                        phase_submit_us,
                    )) => {
                        let post_timer = Instant::now();
                        {
                            let mut state = self.app_state.lock().unwrap();
                            state.vis_picker_scroll_to_cursor = false;
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
                            let post_us = post_timer.elapsed().as_micros() as f32;
                            state.stats.phase_lock_update_us =
                                state.stats.phase_lock_update_us * 0.9 + phase_lock_update_us * 0.1;
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
                            state.stats.phase_post_us =
                                state.stats.phase_post_us * 0.9 + post_us * 0.1;
                        }

                        match action {
                            EngineAction::OpenFile => {
                                self.touch_controller.reset();
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
                            EngineAction::SetAppendToPlaylist(val) => {
                                self.app_state.lock().unwrap().append_to_playlist = val;
                            }
                            EngineAction::SetPassthrough(val) => {
                                self.app_state.lock().unwrap().passthrough_enabled = val;
                            }
                            EngineAction::SetSplitRatio(val) => {
                                self.app_state.lock().unwrap().panel_split_ratio = val;
                            }
                            EngineAction::ScrubPreview(pct, delta) => {
                                let mut state = self.app_state.lock().unwrap();
                                let target = (state.duration_seconds * pct as f64)
                                    .clamp(0.0, state.duration_seconds);
                                state.scrub_target_seconds = Some(target);
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
                                let mut state = self.app_state.lock().unwrap();
                                state.scrub_target_seconds = None;
                            }
                            EngineAction::Seek(pct) => {
                                let mut state = self.app_state.lock().unwrap();
                                let target = (pct as f64 * state.duration_seconds)
                                    .clamp(0.0, state.duration_seconds);
                                state.seek_request = Some(target);
                                state.scrub_target_seconds = None;
                                state.track_ended = false;
                            }
                            EngineAction::SetForceStereo(force) => {
                                self.app_state.lock().unwrap().force_stereo_downmix = force;
                            }
                            EngineAction::ToggleAudioTrackInMix(track_idx) => {
                                let mut state = self.app_state.lock().unwrap();
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
                                    if state.audio_track_volumes.len() != state.audio_tracks.len() {
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
                                    state.audio_mix_request = Some(mix.clone());
                                    crate::android::log_android(
                                        3,
                                        &format!(
                                            "[RustTracker] ToggleAudioTrackInMix: track_idx={}, active={:?}",
                                            track_idx, state.active_audio_tracks
                                        ),
                                    );
                                }
                            }
                            EngineAction::SetAudioTrackVolume(track_idx, volume) => {
                                let mut state = self.app_state.lock().unwrap();
                                if track_idx < state.audio_tracks.len() {
                                    if state.audio_track_volumes.len() != state.audio_tracks.len() {
                                        state.audio_track_volumes =
                                            vec![1.0; state.audio_tracks.len()];
                                    }
                                    state.audio_track_volumes[track_idx] = volume.clamp(0.0, 1.0);
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
                                let mut state = self.app_state.lock().unwrap();
                                state.multi_track_mix_mode = tracks.len() > 1;
                                state.active_audio_tracks =
                                    tracks.iter().map(|(idx, _)| *idx).collect();
                                if state.audio_track_volumes.len() != state.audio_tracks.len() {
                                    state.audio_track_volumes = vec![1.0; state.audio_tracks.len()];
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
                                crate::android::log_android(
                                    3,
                                    &format!(
                                        "[RustTracker] SetAudioMixTracks: tracks={:?}",
                                        tracks
                                    ),
                                );
                                state.audio_mix_request = Some(tracks);
                            }
                            EngineAction::VisPickerSelect(idx) => {
                                let mut state = self.app_state.lock().unwrap();
                                if idx < VISUALIZERS.len() {
                                    state.vis_user_overridden[idx] = true;
                                    state.current_visualizer_idx = idx;
                                    state.visualizer_mode = VISUALIZERS[idx].id;
                                    state.osd_text = Some(VISUALIZERS[idx].name.to_string());
                                    state.osd_timer = 2.0;
                                    state.show_vis_picker = false;
                                }
                            }
                            EngineAction::ToggleVisPicker => {
                                let mut state = self.app_state.lock().unwrap();
                                state.show_vis_picker = !state.show_vis_picker;
                                if state.show_vis_picker {
                                    state.vis_picker_cursor = state.current_visualizer_idx;
                                    state.vis_picker_scroll_to_cursor = true;
                                }
                            }
                            EngineAction::VisPickerToggleEnabled(idx) => {
                                let mut state = self.app_state.lock().unwrap();
                                if idx != 0 && idx < state.vis_enabled.len() {
                                    state.vis_enabled[idx] = !state.vis_enabled[idx];
                                    state.vis_user_overridden[idx] = true;
                                }
                            }
                            EngineAction::VisPickerSetCursor(idx) => {
                                self.app_state.lock().unwrap().vis_picker_cursor = idx;
                            }
                            EngineAction::VisPickerEnableAll => {
                                let mut state = self.app_state.lock().unwrap();
                                for enabled in &mut state.vis_enabled {
                                    *enabled = true;
                                }
                                for overridden in &mut state.vis_user_overridden {
                                    *overridden = true;
                                }
                            }
                            EngineAction::VisPickerEnableNone => {
                                let mut state = self.app_state.lock().unwrap();
                                for (i, enabled) in state.vis_enabled.iter_mut().enumerate() {
                                    *enabled = i == 0;
                                }
                            }
                            EngineAction::OpenUrlDialog => {
                                let mut state = self.app_state.lock().unwrap();
                                state.is_url_dialog_open = true;
                                state.focus_url_input = true;
                                win.set_ime_allowed(true);
                            }
                            EngineAction::ClearFocusUrlInput => {
                                self.app_state.lock().unwrap().focus_url_input = false;
                            }
                            EngineAction::CloseUrlDialog => {
                                self.app_state.lock().unwrap().is_url_dialog_open = false;
                                win.set_ime_allowed(false);
                            }
                            EngineAction::SetUrlInput(url) => {
                                self.app_state.lock().unwrap().url_input_text = url;
                            }
                            EngineAction::EditUrl(url) => {
                                let mut state = self.app_state.lock().unwrap();
                                state.url_input_text = url;
                                state.focus_url_input = true;
                                win.set_ime_allowed(true);
                            }
                            EngineAction::LoadUrl(url) => {
                                let mut final_url = url;
                                if !final_url.starts_with("http://")
                                    && !final_url.starts_with("https://")
                                {
                                    final_url = format!("https://{}", final_url);
                                }
                                win.set_ime_allowed(false);
                                {
                                    let mut state = self.app_state.lock().unwrap();
                                    state.is_url_dialog_open = false;
                                    state.url_input_text.clear();
                                    state.playlist = vec![final_url.clone()];
                                    state.playlist_index = 0;
                                    state.load_request = Some(final_url.clone());
                                    state.file_loaded = true;
                                    if let Some(idx) =
                                        state.url_history.iter().position(|(u, _)| u == &final_url)
                                    {
                                        let item = state.url_history.remove(idx);
                                        state.url_history.push(item);
                                    } else {
                                        state
                                            .url_history
                                            .push((final_url, "Network Stream".to_string()));
                                    }
                                    let mut out = String::new();
                                    for (u, t) in &state.url_history {
                                        out.push_str(&format!("{}|{}\n", u, t));
                                    }
                                    let _ =
                                        std::fs::write(crate::state::get_history_file_path(), out);
                                }
                            }
                            EngineAction::SetAudioDevice(dev) => {
                                let mut state = self.app_state.lock().unwrap();
                                state.selected_audio_device = Some(dev.clone());
                                state.audio_device_change_request = Some(dev);
                            }
                            EngineAction::None => {}
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
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.is_suspended {
            if let Some(ref win) = self.window {
                win.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
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
        scale_factor: f32,
    ) -> Option<EngineAction> {
        let mut state = app_state.lock().unwrap();
        let sf = scale_factor.max(1.0);
        match gesture {
            // Top pane swipe in portrait: cycle HUD tabs (Channels -> Heatmap/Lyrics -> Info -> Video)
            TouchGesture::TopPaneSwipeLeft => {
                if state.video_mode == 3 {
                    state.video_mode = 0;
                }
                state.mobile_hud_tab = state.mobile_hud_tab.next(has_video);
                state.osd_text = Some(match state.mobile_hud_tab {
                    crate::state::MobileHudTab::Channels => "Channels".to_string(),
                    crate::state::MobileHudTab::Heatmap => {
                        if state.lyrics.is_some() {
                            "Lyrics".to_string()
                        } else {
                            "Heatmap".to_string()
                        }
                    }
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
                    crate::state::MobileHudTab::Heatmap => {
                        if state.lyrics.is_some() {
                            "Lyrics".to_string()
                        } else {
                            "Heatmap".to_string()
                        }
                    }
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
                    crate::state::MobileHudTab::Heatmap => {
                        if state.lyrics.is_some() {
                            "Lyrics".to_string()
                        } else {
                            "Heatmap".to_string()
                        }
                    }
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
                    crate::state::MobileHudTab::Heatmap => {
                        if state.lyrics.is_some() {
                            "Lyrics".to_string()
                        } else {
                            "Heatmap".to_string()
                        }
                    }
                    crate::state::MobileHudTab::Info => "Track Info".to_string(),
                    crate::state::MobileHudTab::Video => "Video Stream".to_string(),
                });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::SingleTap { x, y } => {
                let is_portrait = window_width < window_height;

                // Check if top right [📂 OPEN FILE] button was tapped in Track Info pane
                let in_info_hdr = if is_portrait {
                    state.mobile_hud_tab == crate::state::MobileHudTab::Info
                        && x > window_width * 0.65
                } else {
                    state.show_hud && state.video_mode == 0 && x > window_width * 0.82
                };
                if in_info_hdr && y < (115.0 * sf) {
                    return Some(EngineAction::OpenFile);
                }

                // If no file loaded, keep splash screen and HUD active
                if !state.file_loaded {
                    return None;
                }

                // Restrict pause/play single tap functionality to the bottom panel only
                let min_h_pixels = 220.0f32 * sf;
                let top_panel_height = (window_height * state.panel_split_ratio).clamp(
                    min_h_pixels,
                    (window_height - min_h_pixels).max(min_h_pixels),
                );
                let in_bottom_panel = !state.show_hud || y >= (top_panel_height + 24.0 * sf);

                if in_bottom_panel {
                    state.is_paused = !state.is_paused;
                    if !state.is_paused {
                        state.track_ended = false;
                        if state.duration_seconds > 0.0
                            && state.current_seconds >= state.duration_seconds - 0.15
                        {
                            state.seek_request = Some(0.0);
                            state.current_seconds = 0.0;
                        }
                    }
                    state.osd_text = Some(if state.is_paused {
                        "Paused".to_string()
                    } else {
                        "Playing".to_string()
                    });
                    state.osd_timer = 1.5;
                }
                None
            }
            TouchGesture::TwoFingerDoubleTap | TouchGesture::TwoFingerTap => {
                if state.audio_tracks.len() > 1 {
                    let next_track = (state.selected_audio_track + 1) % state.audio_tracks.len();
                    state.selected_audio_track = next_track;
                    state.active_audio_tracks = vec![next_track];
                    state.audio_track_request = Some(next_track);
                    state.audio_mix_request = Some(vec![(next_track, 1.0)]);
                    let track_title = state
                        .audio_tracks
                        .get(next_track)
                        .map(|t| t.title.clone())
                        .unwrap_or_default();
                    let title_display = if track_title.to_lowercase().starts_with("track") {
                        track_title
                    } else {
                        format!("Track {}: {}", next_track + 1, track_title)
                    };
                    state.osd_text = Some(title_display.clone());
                    state.osd_timer = 2.0;
                    crate::android::log_android(
                        3,
                        &format!(
                            "[RustTracker] TwoFingerTap: Switched to track {} ({})",
                            next_track, title_display
                        ),
                    );
                } else {
                    state.osd_text = Some("1 Audio Track Present".to_string());
                    state.osd_timer = 1.5;
                }
                None
            }
            TouchGesture::ThreeFingerTap => {
                state.show_stats = !state.show_stats;
                state.osd_text = Some(if state.show_stats {
                    "Engine Stats: Visible".to_string()
                } else {
                    "Engine Stats: Hidden".to_string()
                });
                state.osd_timer = 1.5;
                None
            }
            TouchGesture::DoubleTap { x: _, y } => {
                // Double tap toggles HUD visibility when in the bottom panel (or anywhere if HUD is hidden)
                let min_h_pixels = 220.0f32 * sf;
                let top_panel_height = (window_height * state.panel_split_ratio).clamp(
                    min_h_pixels,
                    (window_height - min_h_pixels).max(min_h_pixels),
                );
                let in_bottom_panel = !state.show_hud || y >= top_panel_height;

                if in_bottom_panel {
                    state.show_hud = !state.show_hud;
                    state.osd_text = Some(if state.show_hud {
                        "HUD: Visible".to_string()
                    } else {
                        "HUD: Hidden".to_string()
                    });
                    state.osd_timer = 1.5;
                }
                None
            }
            TouchGesture::Scrub { pct } => {
                let target =
                    (pct as f64 * state.duration_seconds).clamp(0.0, state.duration_seconds);
                state.seek_request = Some(target);
                state.scrub_target_seconds = Some(target);
                state.track_ended = false;
                None
            }
            TouchGesture::ScrubEnd => {
                state.scrub_target_seconds = None;
                None
            }
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
            state.spectrum_history.clear();
            for _ in 0..120 {
                state.spectrum_history.push_back(vec![0.0; 1024]);
            }
            state.raw_channel_vus.fill(0.0);
            state.channel_vus.fill(0.0);
            state.peak_vus.fill(0.0);
            state.raw_spectrum_data.fill(0.0);
            state.spectrum_data.fill(0.0);
            state.fire_heat.fill(0.0);
        }
        if let Some(engine) = self.engine.as_mut() {
            engine.clear_video_state();
        }

        match crate::audio::start_audio_thread(path, false, Arc::clone(&self.app_state)) {
            Ok(stream) => {
                log_android(
                    3,
                    &format!("Successfully started audio thread for {}", path),
                );
                eprintln!(
                    "[RustTracker] Successfully started audio thread for {}",
                    path
                );
                let mut state = self.app_state.lock().unwrap();
                state.file_loaded = true;
                state.is_paused = false;
                state.track_ended = false;
                let raw_name = if path.starts_with("http://") || path.starts_with("https://") {
                    path.to_string()
                } else {
                    std::path::Path::new(path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                };
                let clean_name = if let Some(rest) = raw_name.strip_prefix("track_")
                    && let Some(idx) = rest.find('_')
                    && idx > 0
                    && rest[..idx].chars().all(|c| c.is_ascii_digit())
                {
                    rest[idx + 1..].to_string()
                } else {
                    raw_name
                };
                state.song_title = clean_name;
                state.playlist = vec![path.to_string()];
                state.playlist_index = 0;
                state.lyrics = crate::lyrics::load_lyrics_for_file(path).map(std::sync::Arc::new);
                state.spectrum_history.clear();
                for _ in 0..120 {
                    state.spectrum_history.push_back(vec![0.0; 1024]);
                }
                state.raw_channel_vus.fill(0.0);
                state.channel_vus.fill(0.0);
                state.peak_vus.fill(0.0);
                state.raw_spectrum_data.fill(0.0);
                state.spectrum_data.fill(0.0);
                state.fire_heat.fill(0.0);
                if let Some(l) = &state.lyrics {
                    log_android(
                        3,
                        &format!("Loaded {} lines from {:?}", l.lines.len(), l.file_name),
                    );
                    eprintln!(
                        "[RustTracker Lyrics] Loaded {} lines from {:?}",
                        l.lines.len(),
                        l.file_name
                    );
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
    fn __android_log_write(
        prio: i32,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> i32;
    fn pipe(pipefd: *mut i32) -> i32;
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn close(fd: i32) -> i32;
}

pub fn log_android(prio: i32, msg: &str) {
    if let Ok(c_msg) = std::ffi::CString::new(msg) {
        unsafe {
            __android_log_write(prio, c"RustTracker".as_ptr(), c_msg.as_ptr());
        }
    }
}

static STDIO_REDIRECTED: std::sync::Once = std::sync::Once::new();

pub fn init_android_stdio_redirection() {
    STDIO_REDIRECTED.call_once(|| unsafe {
        let mut pfd = [0i32; 2];
        if pipe(pfd.as_mut_ptr()) == 0 {
            dup2(pfd[1], 1); // redirect stdout
            dup2(pfd[1], 2); // redirect stderr
            close(pfd[1]);

            use std::os::fd::FromRawFd;
            let read_fd = pfd[0];
            let _ = std::thread::Builder::new()
                .name("LogcatRedirect".to_string())
                .spawn(move || {
                    use std::io::BufRead;
                    let file = std::fs::File::from_raw_fd(read_fd);
                    let reader = std::io::BufReader::new(file);
                    for l in reader.lines().map_while(Result::ok) {
                        log_android(3, &l);
                    }
                });
        }
    });
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    init_android_stdio_redirection();

    if let Some(data_path) = app.internal_data_path() {
        let _ = crate::state::CACHE_DIR_OVERRIDE.set(data_path);
    }

    std::panic::set_hook(Box::new(|panic_info| {
        let msg = format!("[PANIC] {}", panic_info);
        log_android(6, &msg);
    }));

    log_android(
        3,
        "Starting RustTracker on Android with Touch Gestures, SAF, and Audio Engine...",
    );
    eprintln!("[RustTracker] Starting on Android with Touch Gestures, SAF, and Audio Engine...");

    let event_loop: EventLoop<AppCustomEvent> = match EventLoop::<AppCustomEvent>::with_user_event()
        .with_android_app(app.clone())
        .build()
    {
        Ok(el) => el,
        Err(e) => {
            let err_msg = format!(
                "[RustTracker] EventLoop creation failed: {:?}. Exiting process to allow clean restart.",
                e
            );
            log_android(6, &err_msg);
            eprintln!("{}", err_msg);
            std::process::exit(0);
        }
    };

    let proxy = event_loop.create_proxy();
    *EVENT_LOOP_PROXY
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = Some(proxy);

    let app_state = Arc::new(Mutex::new(AppState::new("RustTracker Mobile".to_string())));
    let _ = GLOBAL_APP_STATE.set(Arc::clone(&app_state));
    let egui_ctx = create_egui_context();

    event_loop.set_control_flow(ControlFlow::Wait);

    let mut android_app = AndroidRustTrackerApp {
        android_app: app,
        app_state,
        window: None,
        engine: None,
        egui_ctx,
        egui_state: None,
        file_dialog: egui_file_dialog::FileDialog::new(),
        touch_controller: TouchGestureController::new(),
        active_stream: None,
        last_frame_time: Instant::now(),
        frame_dt_ring: [1.0 / 60.0; 60],
        frame_dt_idx: 0,
        display_refresh_hz: 60.0,
        is_suspended: false,
        frame_count: 0,
    };

    let _ = event_loop.run_app(&mut android_app);

    *EVENT_LOOP_PROXY
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = None;
    eprintln!("[RustTracker] android_main thread exited cleanly, terminating process.");
    std::process::exit(0);
}
