use std::collections::VecDeque;

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
    pub show_hud: bool,
    pub max_frequency: f32,
}

impl AppState {
    pub fn new(title: String) -> Self {
        let mut history = VecDeque::with_capacity(120);
        for _ in 0..120 {
            history.push_back(vec![0.0; 512]);
        }

        Self {
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
            show_hud: false,
            max_frequency: 10000.0,
        }
    }
}
