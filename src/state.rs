use std::collections::VecDeque;

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
}

pub const VISUALIZERS: &[VisualizerDef] = &[
    VisualizerDef { id: 0, name: "Frequency Spectrum", filename: "vis_spectrum.wgsl", description: "Standard 2D FFT spectrum analyzer", requires_history: false, requires_fire: false, requires_resynth: false },
    VisualizerDef { id: 2, name: "CRT Oscilloscope", filename: "vis_oscilloscope.wgsl", description: "2D glowing CRT wave trace", requires_history: true, requires_fire: false, requires_resynth: false },
    VisualizerDef { id: 7, name: "3D CRT Oscilloscope", filename: "vis_3doscilloscope.wgsl", description: "3D waterfall history of waveform", requires_history: true, requires_fire: false, requires_resynth: false },
    VisualizerDef { id: 8, name: "3D Freq Oscilloscope", filename: "vis_3doscilloscope_freq.wgsl", description: "3D topographical frequency view", requires_history: false, requires_fire: false, requires_resynth: true },
    VisualizerDef { id: 1, name: "Retro Fire", filename: "vis_flame.wgsl", description: "Demoscene pixel fire with CRT filter", requires_history: false, requires_fire: true, requires_resynth: false },
    VisualizerDef { id: 6, name: "Fire Simulation", filename: "vis_firesim.wgsl", description: "Multi-channel procedural fire simulation", requires_history: false, requires_fire: true, requires_resynth: false },
    VisualizerDef { id: 3, name: "Spatial Vectors", filename: "vis_spatial.wgsl", description: "Multi-channel spatial audio map", requires_history: false, requires_fire: false, requires_resynth: false },
    VisualizerDef { id: 4, name: "Chrome Ferrofluid", filename: "vis_ferrofluid.wgsl", description: "Raymarched liquid metal simulation", requires_history: false, requires_fire: false, requires_resynth: false },
    VisualizerDef { id: 5, name: "Neon Corridor", filename: "vis_neon.wgsl", description: "Raymarched neon sci-fi tunnel", requires_history: false, requires_fire: false, requires_resynth: false },
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
    pub file_loaded: bool,
    pub video_info: Option<String>,
    pub show_stats: bool,
    pub stats: PerformanceStats,
    pub current_fps: f32,
    pub playlist: Vec<String>,
    pub playlist_index: usize,
    pub track_ended: bool,
    pub visualizer_mode: u32,
    pub current_visualizer_idx: usize,
    pub video_frame_rx: Option<crossbeam_channel::Receiver<VideoFrame>>,
    pub free_video_frame_tx: Option<crossbeam_channel::Sender<VideoFrame>>,
    pub is_file_picker_open: bool,
    pub open_file_request: bool,
    pub egui_gamepad_events: Vec<egui::Event>,
    pub force_stereo_downmix: bool,
    pub panel_split_ratio: f32,
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
            stats: PerformanceStats::default(),
            current_fps: 0.0,
            playlist: Vec::new(),
            playlist_index: 0,
            track_ended: false,
            visualizer_mode: VISUALIZERS[0].id,
            current_visualizer_idx: 0,
            video_frame_rx: None,
            free_video_frame_tx: None,
            video_mode: 0,
            is_file_picker_open: false,
            open_file_request: false,
            egui_gamepad_events: Vec::new(),
            force_stereo_downmix: is_steam_deck,
            panel_split_ratio: 0.5,
        }
    }
}
