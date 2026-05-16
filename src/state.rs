use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamepadType {
    Xbox,
    PlayStation,
    Nintendo,
    SteamDeck,
}

#[derive(Clone, Default, Debug)]
pub struct PerformanceStats {
    pub decode_us: f32,
    pub fft_us: f32,
    pub ui_us: f32,
    pub render_us: f32,
    pub shader_us: f32,
    pub fire_us: f32,
    pub gpu_fft_us: f32,
    pub audio_buffer_fill_pct: f32,
    pub video_buffer_fill_pct: f32,
    pub clipping_events: u32,
    pub bitstream_active: bool,
    pub bitstream_format: String,
    
    // Fine-grained frame phase timings (microseconds)
    pub phase_lock_update_us: f32,   // Main loop: lock + smoothing + engine.update()
    pub phase_snapshot_us: f32,      // Main loop: render_snapshot() creation
    pub phase_surface_us: f32,       // render(): get_current_texture()
    pub phase_egui_layout_us: f32,   // render(): egui ctx.run() UI layout
    pub phase_encode_us: f32,        // render(): GPU command encoding (compute + render passes)
    #[allow(dead_code)]
    pub phase_submit_us: f32,        // render(): queue.submit() + present
    pub phase_post_us: f32,          // Main loop: post-render state writeback
}

#[derive(Clone)]
pub struct VideoFrame {
    pub pts: f64,
    pub width: u32,
    pub height: u32,
    pub y_plane: Vec<u8>,
    pub u_plane: Vec<u8>,
    pub v_plane: Vec<u8>,
    pub y_stride: usize,
    pub u_stride: usize,
    pub v_stride: usize,
    
    // HDR Metadata
    pub bit_depth: u8,
    pub color_space: u32,
    pub color_range: u32,
    pub color_trc: u32,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("pts", &self.pts)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bit_depth", &self.bit_depth)
            .field("color_space", &self.color_space)
            .field("color_range", &self.color_range)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct VisualizerDef {
    pub id: u32,
    pub name: &'static str,
    pub filename: &'static str,
    #[allow(dead_code)]
    pub description: &'static str,
    pub requires_history: bool,
    pub requires_fire: bool,
    pub requires_resynth: bool,
    pub requires_ferrofluidsim: bool,
}

pub const VISUALIZERS: &[VisualizerDef] = &[
    VisualizerDef { id: 0, name: "Frequency Spectrum", filename: "vis_spectrum.wgsl", description: "Standard 2D FFT spectrum analyzer", requires_history: false, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 2, name: "CRT Oscilloscope", filename: "vis_oscilloscope.wgsl", description: "2D glowing CRT wave trace", requires_history: true, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 7, name: "3D CRT Oscilloscope", filename: "vis_3doscilloscope.wgsl", description: "3D waterfall history of waveform", requires_history: true, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 8, name: "3D Freq Oscilloscope", filename: "vis_3doscilloscope_freq.wgsl", description: "3D topographical frequency view", requires_history: false, requires_fire: false, requires_resynth: true, requires_ferrofluidsim: false },
    VisualizerDef { id: 1, name: "Retro Fire", filename: "vis_flame.wgsl", description: "Demoscene pixel fire with CRT filter", requires_history: false, requires_fire: true, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 6, name: "Fire Simulation", filename: "vis_firesim.wgsl", description: "Multi-channel procedural fire simulation", requires_history: false, requires_fire: true, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 9, name: "Solar Flare", filename: "vis_solar.wgsl", description: "Audio-reactive raymarched sun", requires_history: true, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 3, name: "Spatial Vectors", filename: "vis_spatial.wgsl", description: "Multi-channel spatial audio map", requires_history: false, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 4, name: "Chrome Ferrofluid", filename: "vis_ferrofluid.wgsl", description: "Raymarched liquid metal simulation", requires_history: false, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
    VisualizerDef { id: 10, name: "Ferrofluid Particle Sim", filename: "vis_ferrofluidsim.wgsl", description: "Compute physics droplet simulation", requires_history: false, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: true },
    VisualizerDef { id: 5, name: "Neon Corridor", filename: "vis_neon.wgsl", description: "Raymarched neon sci-fi tunnel", requires_history: false, requires_fire: false, requires_resynth: false, requires_ferrofluidsim: false },
];

#[derive(Debug, Clone)]
pub struct AppState {
    pub song_title: String,
    pub artist: String,
    pub module_type: String,
    pub duration_seconds: f64,
    pub current_seconds: f64,
    pub seek_request: Option<f64>,
    pub seek_epoch: u64,
    pub is_paused: bool,
    pub bpm: i32,
    pub speed: i32,
    pub num_channels: i32,
    pub hardware_channels: i32,
    pub raw_channel_vus: Vec<f32>,
    pub channel_vus: Vec<f32>,
    pub peak_vus: Vec<f32>,
    pub raw_spectrum_data: Vec<f32>,
    pub spectrum_data: Vec<f32>,
    pub spectrum_peaks: Vec<f32>,
    pub spectrum_history: VecDeque<Vec<f32>>,
    pub waveform_history: VecDeque<Vec<f32>>,
    pub raw_waveform: Vec<f32>,
    pub raw_audio_channels: Vec<Vec<f32>>,
    pub fire_heat: Vec<f32>,
    pub show_hud: bool,
    pub gpu_fft: bool,
    pub max_frequency: f32,
    pub num_samples: i32,
    pub num_instruments: i32,
    pub num_patterns: i32,
    pub current_tracker_order: i32,
    pub current_tracker_row: i32,
    pub tracker_row_history: VecDeque<(i32, i32)>,
    pub current_tracker_row_string: String,
    pub tracker_patterns_by_order: Vec<Vec<String>>,
    pub tracker_channels: Option<i32>,
    pub load_request: Option<String>,
    pub video_mode: u32,
    pub playlist: Vec<String>,
    pub playlist_index: usize,
    pub passthrough_enabled: bool,
    pub file_loaded: bool,
    pub video_info: Option<String>,
    pub show_stats: bool,
    pub show_help: bool,
    pub stats: PerformanceStats,
    pub current_fps: f32,
    pub current_sample_rate: f32,
    pub track_ended: bool,
    pub visualizer_mode: u32,
    pub visual_width: u32,
    pub target_fps: u32,
    pub current_visualizer_idx: usize,
    pub video_frame_rx: Option<crossbeam_channel::Receiver<VideoFrame>>,
    pub free_video_frame_tx: Option<crossbeam_channel::Sender<VideoFrame>>,
    pub is_file_picker_open: bool,
    pub open_file_request: bool,
    pub egui_gamepad_events: Vec<egui::Event>,
    pub force_stereo_downmix: bool,
    pub append_to_playlist: bool,
    pub panel_split_ratio: f32,
    pub gamepad_type: GamepadType,
    pub has_gamepad: bool,
    // Visualizer Picker
    pub show_vis_picker: bool,
    pub vis_picker_cursor: usize,
    pub vis_enabled: Vec<bool>,
    pub osd_text: Option<String>,
    pub osd_timer: f32,
    pub cumulative_scrub: f64,
}

impl AppState {
    pub fn new(title: String) -> Self {
        let mut history = VecDeque::new();
        for _ in 0..120 {
            history.push_back(vec![0.0; 1024]);
        }
        
        let mut wave_history = VecDeque::new();
        for _ in 0..60 {
            wave_history.push_back(vec![0.0; 1024]);
        }

        let is_steam_deck = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
            .map(|s| s.trim().to_lowercase().contains("valve"))
            .unwrap_or(false);

        AppState {
            passthrough_enabled: true,
            file_loaded: false,
            song_title: title,
            artist: "Unknown".to_string(),
            module_type: "Unknown".to_string(),
            duration_seconds: 0.0,
            current_seconds: 0.0,
            seek_request: None,
            seek_epoch: 0,
            is_paused: true, // Start paused to prevent audio playback before UI is ready
            bpm: 0,
            speed: 0,
            num_channels: 0,
            hardware_channels: 0,
            raw_channel_vus: Vec::new(),
            channel_vus: Vec::new(),
            peak_vus: Vec::new(),
            raw_spectrum_data: vec![0.0; 1024],
            spectrum_data: vec![0.0; 1024],
            spectrum_peaks: vec![0.0; 1024],
            spectrum_history: history,
            waveform_history: wave_history,
            raw_waveform: vec![0.0; 1024],
            raw_audio_channels: Vec::new(),
            fire_heat: vec![0.0; 1024],
            show_hud: true,
            gpu_fft: true,
            max_frequency: 10000.0,
            num_samples: 0,
            num_instruments: 0,
            num_patterns: 0,
            current_tracker_order: 0,
            current_tracker_row: 0,
            tracker_row_history: VecDeque::with_capacity(128),
            current_tracker_row_string: String::new(),
            tracker_patterns_by_order: Vec::new(),
            tracker_channels: None,
            load_request: None,

            video_info: None,
            show_stats: false,
            show_help: false,
            stats: PerformanceStats::default(),
            current_fps: 0.0,
            current_sample_rate: 44100.0,
            playlist: Vec::new(),
            playlist_index: 0,
            track_ended: false,
            visualizer_mode: VISUALIZERS[0].id,
            visual_width: 1024,
            target_fps: 144,
            current_visualizer_idx: 0,
            video_frame_rx: None,
            free_video_frame_tx: None,
            video_mode: 0,
            is_file_picker_open: false,
            open_file_request: false,
            egui_gamepad_events: Vec::new(),
            force_stereo_downmix: is_steam_deck,
            append_to_playlist: false,
            panel_split_ratio: 0.5,
            gamepad_type: if is_steam_deck { GamepadType::SteamDeck } else { GamepadType::Xbox },
            has_gamepad: is_steam_deck,
            show_vis_picker: false,
            vis_picker_cursor: 0,
            vis_enabled: vec![true; VISUALIZERS.len()],
            osd_text: None,
            osd_timer: 0.0,
            cumulative_scrub: 0.0,
        }
    }
    
    /// Create a lightweight clone for the render pass, skipping heavy audio data fields
    /// that are only consumed by engine.update() (which runs under the mutex lock).
    /// This saves ~1.3 MB of deep copies per frame vs a full .clone().
    pub fn render_snapshot(&self) -> Self {
        AppState {
            passthrough_enabled: self.passthrough_enabled,
            song_title: self.song_title.clone(),
            artist: self.artist.clone(),
            module_type: self.module_type.clone(),
            duration_seconds: self.duration_seconds,
            current_seconds: self.current_seconds,
            seek_request: None,
            seek_epoch: self.seek_epoch,
            is_paused: self.is_paused,
            visual_width: self.visual_width,
            target_fps: self.target_fps,
            bpm: self.bpm,
            speed: self.speed,
            num_channels: self.num_channels,
            hardware_channels: self.hardware_channels,
            raw_channel_vus: Vec::new(),         // Only used in main.rs smoothing (under lock)
            channel_vus: self.channel_vus.clone(), // Small, needed for VU meters
            peak_vus: self.peak_vus.clone(),       // Small, needed for VU meters
            raw_spectrum_data: Vec::new(),         // Only used in main.rs smoothing (under lock)
            spectrum_data: Vec::new(),             // Only used in engine.update() (under lock)
            spectrum_peaks: Vec::new(),            // Only used in engine.update() (under lock)
            spectrum_history: {
                // Render only checks .len() and [0].len() — provide minimal metadata
                let mut sh = VecDeque::new();
                if let Some(first) = self.spectrum_history.front() {
                    sh.push_back(vec![0.0; first.len()]);
                }
                sh
            },
            waveform_history: VecDeque::new(),     // Only used in engine.update() (under lock)
            raw_waveform: Vec::new(),              // Only used in main.rs smoothing (under lock)
            raw_audio_channels: Vec::new(),        // ~1 MB, only used in engine.update()
            fire_heat: Vec::new(),                 // Only used in engine.update() (under lock)
            show_hud: self.show_hud,
            gpu_fft: self.gpu_fft,
            max_frequency: self.max_frequency,
            num_samples: self.num_samples,
            num_instruments: self.num_instruments,
            num_patterns: self.num_patterns,
            current_tracker_order: self.current_tracker_order,
            current_tracker_row: self.current_tracker_row,
            tracker_row_history: self.tracker_row_history.clone(),
            current_tracker_row_string: self.current_tracker_row_string.clone(),
            tracker_patterns_by_order: self.tracker_patterns_by_order.clone(),
            tracker_channels: self.tracker_channels,
            load_request: None,
            video_mode: self.video_mode,
            file_loaded: self.file_loaded,
            video_info: self.video_info.clone(),
            show_stats: self.show_stats,
            show_help: self.show_help,
            stats: self.stats.clone(),
            current_fps: self.current_fps,
            current_sample_rate: self.current_sample_rate,
            playlist: self.playlist.clone(),
            playlist_index: self.playlist_index,
            track_ended: self.track_ended,
            visualizer_mode: self.visualizer_mode,
            current_visualizer_idx: self.current_visualizer_idx,
            video_frame_rx: self.video_frame_rx.clone(),
            free_video_frame_tx: self.free_video_frame_tx.clone(),
            is_file_picker_open: self.is_file_picker_open,
            open_file_request: false,
            egui_gamepad_events: Vec::new(),
            force_stereo_downmix: self.force_stereo_downmix,
            append_to_playlist: self.append_to_playlist,
            panel_split_ratio: self.panel_split_ratio,
            gamepad_type: self.gamepad_type,
            has_gamepad: self.has_gamepad,
            show_vis_picker: self.show_vis_picker,
            vis_picker_cursor: self.vis_picker_cursor,
            vis_enabled: self.vis_enabled.clone(),
            osd_text: self.osd_text.clone(),
            osd_timer: self.osd_timer,
            cumulative_scrub: self.cumulative_scrub,
        }
    }
}
