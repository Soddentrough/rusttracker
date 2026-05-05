use std::collections::VecDeque;

#[derive(Clone, Default, Debug)]
pub struct PerformanceStats {
    pub decode_us: f32,
    pub fft_us: f32,
    pub ui_us: f32,
    pub render_us: f32,
    pub shader_us: f32,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub song_title: String,
    pub artist: String,
    pub module_type: String,
    pub duration_seconds: f64,
    pub current_seconds: f64,
    pub seek_request: Option<f64>,
    pub is_paused: bool,
    pub bpm: i32,
    pub speed: i32,
    pub num_channels: i32,
    pub raw_channel_vus: Vec<f32>,
    pub channel_vus: Vec<f32>,
    pub peak_vus: Vec<f32>,
    pub raw_spectrum_data: Vec<f32>,
    pub spectrum_data: Vec<f32>,
    pub spectrum_peaks: Vec<f32>,
    pub spectrum_history: VecDeque<Vec<f32>>,
    pub waveform_history: VecDeque<Vec<f32>>,
    pub raw_waveform: Vec<f32>,
    pub fire_heat: Vec<f32>,
    pub show_hud: bool,
    pub max_frequency: f32,
    pub num_samples: i32,
    pub num_instruments: i32,
    pub num_patterns: i32,
    pub current_tracker_order: i32,
    pub current_tracker_row: i32,
    pub tracker_row_history: VecDeque<(i32, i32)>,
    pub tracker_patterns_by_order: Vec<Vec<String>>,
    pub load_request: Option<String>,
    pub file_loaded: bool,
    pub show_stats: bool,
    pub stats: PerformanceStats,
    pub current_fps: f32,
    pub playlist: Vec<String>,
    pub playlist_index: usize,
    pub track_ended: bool,
    pub visualizer_mode: u32,
}

impl AppState {
    pub fn new(title: String) -> Self {
        let mut history = VecDeque::new();
        for _ in 0..120 {
            history.push_back(vec![0.0; 512]);
        }
        
        let mut wave_history = VecDeque::new();
        for _ in 0..60 {
            wave_history.push_back(vec![0.0; 512]);
        }

        AppState {
            file_loaded: !title.is_empty(),
            song_title: title,
            artist: "Unknown".to_string(),
            module_type: "Unknown".to_string(),
            duration_seconds: 0.0,
            current_seconds: 0.0,
            seek_request: None,
            is_paused: false,
            bpm: 0,
            speed: 0,
            num_channels: 0,
            raw_channel_vus: Vec::new(),
            channel_vus: Vec::new(),
            peak_vus: Vec::new(),
            raw_spectrum_data: vec![0.0; 512],
            spectrum_data: vec![0.0; 512],
            spectrum_peaks: vec![0.0; 512],
            spectrum_history: history,
            waveform_history: wave_history,
            raw_waveform: vec![0.0; 512],
            fire_heat: vec![0.0; 512],
            show_hud: true,
            max_frequency: 10000.0,
            num_samples: 0,
            num_instruments: 0,
            num_patterns: 0,
            current_tracker_order: 0,
            current_tracker_row: 0,
            tracker_row_history: VecDeque::with_capacity(128),
            tracker_patterns_by_order: Vec::new(),
            load_request: None,
            show_stats: false,
            stats: PerformanceStats::default(),
            current_fps: 0.0,
            playlist: Vec::new(),
            playlist_index: 0,
            track_ended: false,
            visualizer_mode: 0,
        }
    }
}
