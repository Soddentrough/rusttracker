use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openmpt::module::{Logger, Module};
use rustfft::{FftPlanner, num_complex::Complex};
use std::fs::File;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use rustysynth::{MidiFile, MidiFileSequencer, SoundFont, Synthesizer, SynthesizerSettings};
use midly::{Smf, TrackEventKind, MetaMessage, MidiMessage};

use crate::state::AppState;
use crossbeam_channel::{bounded, Sender, Receiver};

#[allow(dead_code)]
pub enum PlaybackHandle {
    Cpal(cpal::Stream),
    Bitstream(std::thread::JoinHandle<()>, Arc<std::sync::atomic::AtomicBool>),
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        if let PlaybackHandle::Bitstream(_, stop_token) = self {
            stop_token.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

pub struct DspMessage {
    pub audio_data: Vec<f32>,
    pub channel_vus: Vec<f32>,
    pub current_order: i32,
    pub current_row: i32,
    pub bpm: i32,
    pub speed: i32,
    pub current_seconds: f64,
    pub current_row_string: String,
    pub channel_audio_data: Vec<Vec<f32>>,
}

pub fn spawn_dsp_thread(
    rx: Receiver<DspMessage>,
    shared_state: Arc<Mutex<AppState>>,
    sample_rate: u32,
    max_frequency: f32,
    window_size: usize,
) {
    std::thread::spawn(move || {
        let mut binned_data = vec![0.0; 1024];

        // Pre-compute Hann window coefficients
        let hann_window: Vec<f32> = (0..window_size)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (window_size - 1) as f32).cos()))
            .collect();

        // Pre-plan FFT (rustfft caches the twiddle factors)
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(window_size);
        let mut complex_buf = vec![Complex { re: 0.0f32, im: 0.0f32 }; window_size];
        let mut magnitudes = vec![0.0f32; window_size / 2];

        let mut last_waveform_push = Instant::now();

        while let Ok(msg) = rx.recv() {
            let fft_start = Instant::now();

            // Apply Hann window and prepare complex input
            for i in 0..window_size {
                let sample = *msg.audio_data.get(i).unwrap_or(&0.0);
                complex_buf[i] = Complex { re: sample * hann_window[i], im: 0.0 };
            }

            // Run FFT
            fft.process(&mut complex_buf);

            // Compute magnitudes (only first half — up to Nyquist)
            let n_sqrt = (window_size as f32).sqrt();
            for i in 0..window_size / 2 {
                magnitudes[i] = complex_buf[i].norm() / n_sqrt;
            }

            // Logarithmic binning into 1024 display bins
            let resolution = sample_rate as f32 / window_size as f32; // Hz per FFT bin
            let min_freq = 20.0f32;
            let max_f = max_frequency.max(min_freq * 2.0);
            let num_bins = 1024;
            let nyquist = window_size / 2;

            binned_data.fill(0.0);
            for i in 0..num_bins {
                let freq_start = min_freq * (max_f / min_freq).powf(i as f32 / num_bins as f32);
                let freq_end = min_freq * (max_f / min_freq).powf((i + 1) as f32 / num_bins as f32);

                let idx_start = freq_start / resolution;
                let idx_end = freq_end / resolution;

                let mut max_val: f32 = 0.0;

                if idx_end - idx_start >= 1.0 {
                    let start = idx_start.ceil() as usize;
                    let end = idx_end.floor() as usize;
                    for idx in start..=end {
                        if idx < nyquist {
                            max_val = max_val.max(magnitudes[idx]);
                        }
                    }
                } else {
                    let nearest = ((idx_start + idx_end) / 2.0).round() as usize;
                    if nearest < nyquist {
                        max_val = magnitudes[nearest];
                    }
                }

                binned_data[i] = (max_val * 100.0).clamp(0.0, 100.0);
            }
            let fft_elapsed = fft_start.elapsed().as_micros() as f32;

            // Sync to UI state
            if let Ok(mut state) = shared_state.lock() {
                // Decay/smooth the execution stats for readability
                state.stats.fft_us = state.stats.fft_us * 0.9 + fft_elapsed * 0.1;
                
                if state.stats.bitstream_active {
                    if let Some(cap) = rx.capacity() {
                        state.stats.audio_buffer_fill_pct = (rx.len() as f32 / cap as f32) * 100.0;
                    }
                }
                
                state.raw_channel_vus.clear();
                for vu in msg.channel_vus {
                    state.raw_channel_vus.push(vu);
                }
                
                state.raw_spectrum_data.copy_from_slice(&binned_data);
                state.raw_audio_channels = msg.channel_audio_data;
                
                // --- Waveform extraction (Zero-Crossing Edge Trigger) ---
                let visual_width = state.visual_width.max(128).min(4096) as usize;
                let target_fps = state.target_fps.max(30).min(500);
                let waveform_push_interval = std::time::Duration::from_secs_f64(1.0 / target_fps as f64);
                
                // Maintain a constant ~23ms time window regardless of sample rate
                let target_window_samples = (sample_rate as f32 * 0.023).round() as usize;
                let stride = (target_window_samples as f32 / visual_width as f32).max(1.0);
                let actual_window_samples = (visual_width as f32 * stride).ceil() as usize;
                
                // Restrict search to the last ~33ms to ensure a fresh trigger at >= 30 FPS,
                // avoiding the visual "freeze" caused by searching the entire history buffer.
                let search_window = (sample_rate as f32 / 30.0) as usize;
                let search_start = msg.audio_data.len().saturating_sub(actual_window_samples + search_window);
                let search_limit = msg.audio_data.len().saturating_sub(actual_window_samples);
                
                let mut start_idx = search_start;
                let mut best_slope = 0.0;
                
                for i in search_start..search_limit {
                    if msg.audio_data[i] <= 0.0 && msg.audio_data[i + 1] > 0.0 {
                        let slope = msg.audio_data[i + 1] - msg.audio_data[i];
                        if slope > best_slope {
                            best_slope = slope;
                            start_idx = i;
                        }
                    }
                }
                
                if state.raw_waveform.len() != visual_width {
                    state.raw_waveform.resize(visual_width, 0.0);
                }
                
                for i in 0..visual_width {
                    let sample_idx = start_idx + (i as f32 * stride) as usize;
                    state.raw_waveform[i] = msg.audio_data[sample_idx.min(msg.audio_data.len() - 1)];
                }
                
                let now = Instant::now();
                if now.duration_since(last_waveform_push) >= waveform_push_interval {
                    while state.waveform_history.len() >= 144 {
                        state.waveform_history.pop_front();
                    }
                    let wave_clone = state.raw_waveform.clone();
                    state.waveform_history.push_back(wave_clone);
                    last_waveform_push = now;
                }
                
                if msg.bpm != 0 { state.bpm = msg.bpm; }
                if msg.speed != 0 { state.speed = msg.speed; }
                state.current_seconds = msg.current_seconds;
                state.current_tracker_row_string = msg.current_row_string;
                
                if state.current_tracker_order != msg.current_order || state.current_tracker_row != msg.current_row {
                    let cur_order = state.current_tracker_order;
                    let cur_row = state.current_tracker_row;
                    
                    if cur_order == msg.current_order && msg.current_row > cur_row {
                        for r in cur_row..msg.current_row {
                            state.tracker_row_history.push_front((cur_order, r));
                        }
                    } else {
                        state.tracker_row_history.push_front((cur_order, cur_row));
                    }
                    
                    state.tracker_row_history.truncate(128);
                    
                    state.current_tracker_order = msg.current_order;
                    state.current_tracker_row = msg.current_row;
                }
                

            }
        }
    });
}

pub trait AudioSource: Send {
    fn read_frames(&mut self, hardware_channels: usize, sample_rate: u32, output: &mut [f32]) -> usize;
    fn get_duration_seconds(&mut self) -> f64;
    fn get_position_seconds(&mut self) -> f64;
    fn set_position_seconds(&mut self, pos: f64);
    fn get_num_channels(&mut self) -> i32;
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32;
    fn get_artist(&mut self) -> String;
    fn get_type(&mut self) -> String;
    fn get_tempo(&mut self) -> i32;
    fn get_speed(&mut self) -> i32;
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32>;
    fn get_num_samples(&mut self) -> i32;
    fn get_num_instruments(&mut self) -> i32;
    fn get_num_patterns(&mut self) -> i32;
    fn get_current_order(&mut self) -> i32;
    fn get_current_row(&mut self) -> i32;
    fn get_tracker_channels(&mut self) -> Option<i32> { None }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> { Vec::new() }
    fn get_current_row_string(&mut self) -> String { String::new() }
    fn get_video_info(&mut self) -> Option<String> { None }
    fn attach_video_queue(&mut self, _tx: crossbeam_channel::Sender<ffmpeg_next::Packet>) {}
    fn take_video_parameters(&mut self) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational)> { None }
}

// ---------------------------------------------------------
// OpenMPT Tracker Decoder
// ---------------------------------------------------------
pub struct SafeModule(pub Module);
unsafe impl Send for SafeModule {}

struct OpenMptSource {
    module: SafeModule,
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
}

impl AudioSource for OpenMptSource {
    fn read_frames(&mut self, hardware_channels: usize, sample_rate: u32, output: &mut [f32]) -> usize {
        let frames_to_render = output.len() / hardware_channels;

        // The openmpt crate uses Vec::capacity() (not len()) to decide how many
        // frames to render. After Vec::resize(), capacity can exceed length due
        // to Vec's doubling growth strategy, causing the C library to write past
        // the Vec's logical length. Allocate exact-size buffers to avoid this.
        self.left_buf = vec![0.0; frames_to_render];
        self.right_buf = vec![0.0; frames_to_render];

        let frames_read = self.module.0.read_float_stereo(
            sample_rate as i32,
            &mut self.left_buf,
            &mut self.right_buf
        );

        for i in 0..frames_read {
            let l = self.left_buf[i];
            let r = self.right_buf[i];
            
            output[i * hardware_channels] = l;
            if hardware_channels > 1 {
                output[i * hardware_channels + 1] = r;
                for j in 2..hardware_channels {
                    output[i * hardware_channels + j] = 0.0;
                }
            }
        }

        frames_read
    }

    fn get_duration_seconds(&mut self) -> f64 { self.module.0.get_duration_seconds() }
    fn get_position_seconds(&mut self) -> f64 { self.module.0.get_position_seconds() }
    fn set_position_seconds(&mut self, pos: f64) { self.module.0.set_position_seconds(pos); }
    fn get_num_channels(&mut self) -> i32 { 2 }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 { self.module.0.get_current_channel_vu_mono(channel) }
    
    fn get_artist(&mut self) -> String {
        use openmpt::module::metadata::MetadataKey;
        self.module.0.get_metadata(MetadataKey::ModuleArtist).unwrap_or("Unknown".to_string())
    }
    
    fn get_type(&mut self) -> String {
        use openmpt::module::metadata::MetadataKey;
        self.module.0.get_metadata(MetadataKey::TypeExt).unwrap_or("Tracker".to_string())
    }
    
    fn get_tempo(&mut self) -> i32 { self.module.0.get_current_tempo() }
    fn get_speed(&mut self) -> i32 { self.module.0.get_current_speed() }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> { None }
    fn get_num_samples(&mut self) -> i32 { self.module.0.get_num_samples() }
    fn get_num_instruments(&mut self) -> i32 { self.module.0.get_num_instruments() }
    fn get_num_patterns(&mut self) -> i32 { self.module.0.get_num_patterns() }
    fn get_current_order(&mut self) -> i32 { self.module.0.get_current_order() }
    fn get_current_row(&mut self) -> i32 { self.module.0.get_current_row() }
    fn get_tracker_channels(&mut self) -> Option<i32> { Some(self.module.0.get_num_channels()) }

    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        let mut patterns_by_order = Vec::new();
        let num_orders = self.module.0.get_num_orders();
        let num_channels = self.module.0.get_num_channels();
        
        for o in 0..num_orders {
            let mut row_strings = Vec::new();
            if let Some(mut pattern) = self.module.0.get_pattern_by_order(o) {
                let num_rows = pattern.get_num_rows();
                for r in 0..num_rows {
                    if let Some(mut row) = pattern.get_row_by_number(r) {
                        let mut row_str = String::new();
                        for c in 0..num_channels {
                            if let Some(mut cell) = row.get_cell_by_channel(c) {
                                if c != 0 { row_str.push_str(" | "); }
                                row_str.push_str(&cell.get_formatted(0, false));
                            }
                        }
                        row_strings.push(row_str);
                    }
                }
            }
            patterns_by_order.push(row_strings);
        }
        
        patterns_by_order
    }

    fn get_current_row_string(&mut self) -> String {
        let mut row_str = String::new();
        let num_channels = self.module.0.get_num_channels();
        let cur_order = self.module.0.get_current_order();
        let cur_row = self.module.0.get_current_row();
        
        if let Some(mut pattern) = self.module.0.get_pattern_by_order(cur_order) {
            if let Some(mut row) = pattern.get_row_by_number(cur_row) {
                for c in 0..num_channels {
                    if let Some(mut cell) = row.get_cell_by_channel(c) {
                        if c != 0 { row_str.push_str(" | "); }
                        row_str.push_str(&cell.get_formatted(0, false));
                    }
                }
            }
        }
        row_str
    }
}

// ---------------------------------------------------------
// MidiTracker Audio Decoder
// ---------------------------------------------------------
#[derive(Clone)]
struct MidiEvent {
    time_sec: f64,
    channel: i32,
    velocity: u8,
    is_note_on: bool,
}

struct MidiSource {
    sequencer: MidiFileSequencer,
    events: Vec<MidiEvent>,
    event_idx: usize,
    channel_vus: Vec<f32>,
    duration: f64,
    artist: String,
    title: String,
    tracker_data: Vec<Vec<String>>,
    tempo: i32,
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
    midi_file: Arc<MidiFile>,
}

unsafe impl Send for MidiSource {}

impl MidiSource {
    pub fn new(file_path: &str, soundfont_path: &str, sample_rate: i32) -> anyhow::Result<Self> {
        let mut sf2_file = File::open(soundfont_path)?;
        let sf2 = Arc::new(SoundFont::new(&mut sf2_file).map_err(|e| anyhow::anyhow!("SoundFont error: {:?}", e))?);
        let settings = SynthesizerSettings::new(sample_rate);
        let synth = Synthesizer::new(&sf2, &settings).map_err(|e| anyhow::anyhow!("Synth error: {:?}", e))?;
        let mut sequencer = MidiFileSequencer::new(synth);
        
        let mut midi_file = File::open(file_path)?;
        let midi = Arc::new(MidiFile::new(&mut midi_file).map_err(|e| anyhow::anyhow!("Midi parse error: {:?}", e))?);
        
        let duration = midi.get_length();
        sequencer.play(&midi, false);
        
        let data = std::fs::read(file_path)?;
        let smf = Smf::parse(&data).map_err(|e| anyhow::anyhow!("Midly parse error: {:?}", e))?;
        
        let mut artist = String::new();
        let mut title = String::new();
        
        let mut absolute_events = Vec::new();
        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(ticks) => ticks.as_int() as f64,
            _ => 480.0,
        };
        
        for track in &smf.tracks {
            let mut current_tick = 0;
            for event in track {
                current_tick += event.delta.as_int();
                match &event.kind {
                    TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                        if title.is_empty() {
                            title = String::from_utf8_lossy(name).to_string();
                        }
                    }
                    TrackEventKind::Meta(MetaMessage::Text(text)) => {
                        if artist.is_empty() {
                            artist = String::from_utf8_lossy(text).to_string();
                        }
                    }
                    TrackEventKind::Midi { channel, message } => {
                        match message {
                            MidiMessage::NoteOn { key, vel } => {
                                absolute_events.push((current_tick, channel.as_int(), key.as_int(), vel.as_int(), vel.as_int() > 0));
                            }
                            MidiMessage::NoteOff { key, vel } => {
                                absolute_events.push((current_tick, channel.as_int(), key.as_int(), vel.as_int(), false));
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
        
        absolute_events.sort_by_key(|e| e.0);
        
        let tempo = 500000.0;
        let mut current_tick = 0;
        let mut current_time = 0.0;
        
        let mut parsed_events = Vec::new();
        for (tick, channel, _key, vel, is_on) in absolute_events {
            let delta_ticks = tick - current_tick;
            let beats = delta_ticks as f64 / ticks_per_beat;
            let seconds = beats * (tempo / 1000000.0);
            current_time += seconds;
            current_tick = tick;
            
            parsed_events.push(MidiEvent {
                time_sec: current_time,
                channel: channel as i32,
                velocity: vel,
                is_note_on: is_on,
            });
        }
        
        let mut pattern_rows: Vec<Vec<String>> = Vec::new();
        for track in &smf.tracks {
            let mut tick = 0;
            for event in track {
                tick += event.delta.as_int();
                if let TrackEventKind::Midi { channel, message } = &event.kind {
                    if let MidiMessage::NoteOn { key, vel } = message {
                        if *vel > 0 {
                            let row_idx = (tick as f64 / (ticks_per_beat / 4.0)) as usize;
                            while pattern_rows.len() <= row_idx {
                                pattern_rows.push(vec!["... .. ..".to_string(); 16]);
                            }
                            let notes = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
                            let octave = key.as_int() / 12;
                            let note = key.as_int() % 12;
                            let note_name = format!("{}{}", notes[note as usize], octave);
                            let note_str = format!("{} .. {:02X}", note_name, vel.as_int());
                            pattern_rows[row_idx][channel.as_int() as usize] = note_str;
                        }
                    }
                }
            }
        }
        
        let mut final_rows = Vec::new();
        for row in pattern_rows {
            final_rows.push(row.join(" | "));
        }
        let tracker_data = vec![final_rows];
        
        if artist.is_empty() {
            artist = "Unknown MIDI".to_string();
        }
        
        Ok(Self {
            sequencer,
            events: parsed_events,
            event_idx: 0,
            channel_vus: vec![0.0; 16],
            duration,
            artist,
            title,
            tracker_data,
            tempo: 120,
            left_buf: vec![0.0; 8192],
            right_buf: vec![0.0; 8192],
            midi_file: midi,
        })
    }
}

impl AudioSource for MidiSource {
    fn read_frames(&mut self, hardware_channels: usize, _sample_rate: u32, output: &mut [f32]) -> usize {
        let frames_to_render = output.len() / hardware_channels;
        
        if self.left_buf.len() < frames_to_render {
            self.left_buf.resize(frames_to_render, 0.0);
            self.right_buf.resize(frames_to_render, 0.0);
        }
        
        self.sequencer.render(&mut self.left_buf[..frames_to_render], &mut self.right_buf[..frames_to_render]);
        
        for c in 0..16 {
            self.channel_vus[c] = (self.channel_vus[c] - 0.02).max(0.0);
        }
        
        let pos = self.sequencer.get_position();
        while self.event_idx < self.events.len() && self.events[self.event_idx].time_sec <= pos {
            let ev = &self.events[self.event_idx];
            if ev.is_note_on {
                let ch = ev.channel as usize;
                if ch < 16 {
                    self.channel_vus[ch] = (ev.velocity as f32 / 127.0).clamp(0.1, 1.0);
                }
            }
            self.event_idx += 1;
        }
        
        for i in 0..frames_to_render {
            let l = self.left_buf[i];
            let r = self.right_buf[i];
            
            output[i * hardware_channels] = l;
            if hardware_channels > 1 {
                output[i * hardware_channels + 1] = r;
                for j in 2..hardware_channels {
                    output[i * hardware_channels + j] = 0.0;
                }
            }
        }
        
        if self.sequencer.end_of_sequence() {
            0
        } else {
            frames_to_render
        }
    }
    
    fn get_duration_seconds(&mut self) -> f64 { self.duration }
    fn get_position_seconds(&mut self) -> f64 { self.sequencer.get_position() }
    
    fn set_position_seconds(&mut self, pos: f64) {
        if pos < self.get_position_seconds() {
            self.sequencer.play(&self.midi_file, false);
        }
        
        let mut trash_left = vec![0.0; 8192];
        let mut trash_right = vec![0.0; 8192];
        while self.sequencer.get_position() < pos && !self.sequencer.end_of_sequence() {
            self.sequencer.render(&mut trash_left, &mut trash_right);
        }
        
        self.event_idx = 0;
        while self.event_idx < self.events.len() && self.events[self.event_idx].time_sec < pos {
            self.event_idx += 1;
        }
        for c in 0..16 { self.channel_vus[c] = 0.0; }
    }
    
    fn get_num_channels(&mut self) -> i32 { 2 }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        if channel >= 0 && channel < 16 {
            self.channel_vus[channel as usize]
        } else {
            0.0
        }
    }
    
    fn get_artist(&mut self) -> String {
        if !self.title.is_empty() {
            format!("{} - {}", self.artist, self.title)
        } else {
            self.artist.clone()
        }
    }
    
    fn get_type(&mut self) -> String { "MIDI".to_string() }
    fn get_tempo(&mut self) -> i32 { self.tempo }
    fn get_speed(&mut self) -> i32 { 0 }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> { Some(48000) }
    fn get_num_samples(&mut self) -> i32 { 0 }
    fn get_num_instruments(&mut self) -> i32 { 128 }
    fn get_num_patterns(&mut self) -> i32 { 1 }
    
    fn get_current_order(&mut self) -> i32 { 0 }
    fn get_current_row(&mut self) -> i32 {
        (self.sequencer.get_position() * (self.tempo as f64 / 60.0) * 4.0) as i32
    }
    
    fn get_tracker_channels(&mut self) -> Option<i32> { Some(16) }
    
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        self.tracker_data.clone()
    }
    
    fn get_current_row_string(&mut self) -> String {
        let row = self.get_current_row() as usize;
        if self.tracker_data.len() > 0 && row < self.tracker_data[0].len() {
            self.tracker_data[0][row].clone()
        } else {
            vec!["... .. ..".to_string(); 16].join(" | ")
        }
    }
}

// ---------------------------------------------------------
// Symphonia Standard Audio Decoder
// ---------------------------------------------------------
struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_buf: SampleBuffer<f32>,
    buf_pos: usize,
    time_base: f64,
    current_time: f64,
    duration: f64,
    channels: u16,
    channel_vus: Vec<f32>,
    artist: String,
    ext_type: String,
    intrinsic_sample_rate: Option<u32>,
    video_info: Option<String>,
}

impl AudioSource for SymphoniaSource {
    fn read_frames(&mut self, hardware_channels: usize, _sample_rate: u32, output: &mut [f32]) -> usize {
        let mut frames_written = 0;
        let frames_needed = output.len() / hardware_channels;
        self.channel_vus.fill(0.0);

        while frames_written < frames_needed {
            if self.buf_pos >= self.sample_buf.len() {
                match self.format.next_packet() {
                    Ok(packet) => {
                        if packet.track_id() != self.track_id { continue; }
                        match self.decoder.decode(&packet) {
                            Ok(decoded) => {
                                if self.sample_buf.capacity() < decoded.capacity() {
                                    self.sample_buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                                }
                                self.sample_buf.copy_interleaved_ref(decoded);
                                self.buf_pos = 0;
                                self.current_time = packet.ts() as f64 * self.time_base;
                            }
                            Err(_) => break,
                        }
                    }
                    Err(_) => break, // EOF
                }
            }

            let frames_available = (self.sample_buf.len() - self.buf_pos) / self.channels as usize;
            if frames_available == 0 { break; }
            let frames_to_copy = frames_available.min(frames_needed - frames_written);
            let samples = self.sample_buf.samples();

            for i in 0..frames_to_copy {
                let base_idx = self.buf_pos + i * self.channels as usize;
                
                // Track VUs for all native channels
                for c in 0..self.channels as usize {
                    let val = samples[base_idx + c];
                    self.channel_vus[c] = self.channel_vus[c].max(val.abs());
                }
                
                // Matrix Downmix / Remap to hardware_channels
                let out_base = (frames_written + i) * hardware_channels;
                
                if self.channels as usize == hardware_channels {
                    // Direct copy
                    for c in 0..hardware_channels {
                        output[out_base + c] = samples[base_idx + c];
                    }
                } else if hardware_channels == 2 && self.channels > 2 {
                    // Downmix to Stereo
                    // Assuming Ch0=L, Ch1=R, Ch2=C, Ch3=LFE, Ch4=Ls, Ch5=Rs
                    let l = samples[base_idx];
                    let r = samples[base_idx + 1];
                    let c = if self.channels > 2 { samples[base_idx + 2] } else { 0.0 };
                    
                    let mut l_surround = 0.0;
                    let mut r_surround = 0.0;
                    if self.channels >= 6 {
                        l_surround = samples[base_idx + 4];
                        r_surround = samples[base_idx + 5];
                    }
                    
                    // LFE usually discarded in 2.0 downmix
                    output[out_base] = (l + c * 0.707 + l_surround * 0.707).clamp(-1.0, 1.0);
                    output[out_base + 1] = (r + c * 0.707 + r_surround * 0.707).clamp(-1.0, 1.0);
                } else {
                    // Generic fallback (copy what we can, zero the rest)
                    for c in 0..hardware_channels {
                        if c < self.channels as usize {
                            output[out_base + c] = samples[base_idx + c];
                        } else {
                            output[out_base + c] = 0.0;
                        }
                    }
                }
            }
            
            self.buf_pos += frames_to_copy * self.channels as usize;
            frames_written += frames_to_copy;
        }

        frames_written
    }

    fn get_duration_seconds(&mut self) -> f64 { self.duration }
    fn get_position_seconds(&mut self) -> f64 { self.current_time }
    
    fn set_position_seconds(&mut self, pos: f64) {
        let ts = (pos / self.time_base) as u64;
        let seek_res = self.format.seek(
            symphonia::core::formats::SeekMode::Coarse,
            symphonia::core::formats::SeekTo::TimeStamp {
                ts,
                track_id: self.track_id,
            }
        );
        
        if seek_res.is_ok() {
            self.decoder.reset();
            self.current_time = pos;
            self.buf_pos = self.sample_buf.len(); // drain buffer
        } else {
            // If TimeStamp seek fails, fallback to Time seek just in case
            if let Ok(_) = self.format.seek(
                symphonia::core::formats::SeekMode::Coarse,
                symphonia::core::formats::SeekTo::Time {
                    time: symphonia::core::units::Time::from(pos),
                    track_id: Some(self.track_id),
                }
            ) {
                self.decoder.reset();
                self.current_time = pos;
                self.buf_pos = self.sample_buf.len();
            }
        }
    }
    
    fn get_num_channels(&mut self) -> i32 { self.channels as i32 }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 { self.channel_vus.get(channel as usize).cloned().unwrap_or(0.0) }
    fn get_artist(&mut self) -> String { self.artist.clone() }
    fn get_type(&mut self) -> String { self.ext_type.clone() }
    fn get_tempo(&mut self) -> i32 { 0 }
    fn get_speed(&mut self) -> i32 { 0 }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> { self.intrinsic_sample_rate }
    fn get_num_samples(&mut self) -> i32 { 0 }
    fn get_num_instruments(&mut self) -> i32 { 0 }
    fn get_num_patterns(&mut self) -> i32 { 0 }
    fn get_current_order(&mut self) -> i32 { 0 }
    fn get_current_row(&mut self) -> i32 { 0 }

    fn get_video_info(&mut self) -> Option<String> { self.video_info.clone() }
}

// ---------------------------------------------------------
// FFmpeg Native Decoder
// ---------------------------------------------------------
struct FfmpegSource {
    ictx: ffmpeg_next::format::context::Input,
    decoder: ffmpeg_next::decoder::Audio,
    stream_index: usize,
    resampler: ffmpeg_next::software::resampling::Context,
    
    sample_buf: Vec<f32>,
    buf_pos: usize,
    
    channels: u16,
    time_base: f64,
    current_time: f64,
    duration: f64,
    artist: String,
    ext_type: String,
    intrinsic_sample_rate: u32,
    video_info: Option<String>,
    channel_vus: Vec<f32>,
    video_stream_index: Option<usize>,
    video_tx: Option<crossbeam_channel::Sender<ffmpeg_next::Packet>>,
    video_params: Option<ffmpeg_next::codec::Parameters>,
    video_time_base: Option<ffmpeg_next::Rational>,
}

impl FfmpegSource {
    fn get_next_frame(&mut self) -> bool {
        let mut decoded = ffmpeg_next::frame::Audio::empty();
        
        // 1. Try to receive a frame from already-buffered packets
        if self.decoder.receive_frame(&mut decoded).is_ok() {
            let mut resampled = ffmpeg_next::frame::Audio::empty();
            match self.resampler.run(&decoded, &mut resampled) {
                Ok(_) => {
                    let data = resampled.plane::<f32>(0);
                    let actual_len = resampled.samples() * resampled.channels() as usize;
                    let actual_data = unsafe { std::slice::from_raw_parts(data.as_ptr(), actual_len) };
                    
                    self.sample_buf.clear();
                    self.sample_buf.extend_from_slice(actual_data);
                    self.buf_pos = 0;
                    return true;
                }
                Err(_) => {
                    // Recreate resampler if input format/layout changed mid-stream
                    if let Ok(new_resampler) = ffmpeg_next::software::resampling::context::Context::get(
                        decoded.format(),
                        decoded.channel_layout(),
                        decoded.rate(),
                        ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                        self.resampler.output().channel_layout,
                        self.resampler.output().rate,
                    ) {
                        self.resampler = new_resampler;
                    }
                }
            }
        }
        
        // 2. Decoder needs more data, read packets
        for (stream, packet) in self.ictx.packets() {
            if Some(stream.index()) == self.video_stream_index {
                if let Some(tx) = &self.video_tx {
                    let _ = tx.try_send(packet.clone());
                }
            } else if stream.index() == self.stream_index {
                // Send the packet to the decoder
                if let Err(_e) = self.decoder.send_packet(&packet) {
                    // Packet might be rejected if decoder is full, but we just drained it above.
                    // Or it could be a decode error. We continue to see if we can receive.
                }
                
                // Now try to receive a frame
                if self.decoder.receive_frame(&mut decoded).is_ok() {
                    let mut resampled = ffmpeg_next::frame::Audio::empty();
                    match self.resampler.run(&decoded, &mut resampled) {
                        Ok(_) => {
                            let data = resampled.plane::<f32>(0);
                            let actual_len = resampled.samples() * resampled.channels() as usize;
                            let actual_data = unsafe { std::slice::from_raw_parts(data.as_ptr(), actual_len) };
                            
                            self.sample_buf.clear();
                            self.sample_buf.extend_from_slice(actual_data);
                            self.buf_pos = 0;
                            if let Some(pts) = packet.pts() {
                                self.current_time = pts as f64 * self.time_base;
                            }
                            return true;
                        }
                        Err(_) => {
                            if let Ok(new_resampler) = ffmpeg_next::software::resampling::context::Context::get(
                                decoded.format(),
                                decoded.channel_layout(),
                                decoded.rate(),
                                ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                                self.resampler.output().channel_layout,
                                self.resampler.output().rate,
                            ) {
                                self.resampler = new_resampler;
                            }
                        }
                    }
                }
            }
        }
        
        false
    }
}

impl AudioSource for FfmpegSource {
    fn read_frames(&mut self, hardware_channels: usize, _sample_rate: u32, output: &mut [f32]) -> usize {
        let mut frames_written = 0;
        let frames_needed = output.len() / hardware_channels;
        self.channel_vus.fill(0.0);

        while frames_written < frames_needed {
            let frames_available = (self.sample_buf.len() - self.buf_pos) / self.channels as usize;
            
            if frames_available == 0 {
                if !self.get_next_frame() {
                    break;
                }
                continue;
            }

            let frames_to_copy = std::cmp::min(frames_needed - frames_written, frames_available);

            let out_base = frames_written * hardware_channels;
            let in_base = self.buf_pos;

            for f in 0..frames_to_copy {
                let out_idx = out_base + f * hardware_channels;
                let in_idx = in_base + f * self.channels as usize;

                // Track VUs
                for c in 0..self.channels as usize {
                    let val = self.sample_buf[in_idx + c];
                    if val.abs() > self.channel_vus[c] {
                        self.channel_vus[c] = val.abs();
                    }
                }

                if self.channels as usize == hardware_channels {
                    for c in 0..hardware_channels {
                        output[out_idx + c] = self.sample_buf[in_idx + c];
                    }
                } else if hardware_channels == 2 && self.channels >= 6 {
                    // Downmix 5.1/7.1 to stereo
                    let l = self.sample_buf[in_idx];
                    let r = self.sample_buf[in_idx + 1];
                    let c = self.sample_buf[in_idx + 2];
                    
                    let l_surround = self.sample_buf[in_idx + 4];
                    let r_surround = self.sample_buf[in_idx + 5];
                    
                    // Normalize by 1.5 to prevent hard clipping when summing loud correlated channels
                    output[out_idx] = ((l + c * 0.707 + l_surround * 0.707) / 1.5).clamp(-1.0, 1.0);
                    output[out_idx + 1] = ((r + c * 0.707 + r_surround * 0.707) / 1.5).clamp(-1.0, 1.0);
                } else {
                    // Fallback generic mapping
                    for c in 0..hardware_channels {
                        if c < self.channels as usize {
                            output[out_idx + c] = self.sample_buf[in_idx + c];
                        } else {
                            output[out_idx + c] = 0.0;
                        }
                    }
                }
            }
            
            self.buf_pos += frames_to_copy * self.channels as usize;
            frames_written += frames_to_copy;
        }

        frames_written
    }

    fn get_duration_seconds(&mut self) -> f64 { self.duration }
    fn get_position_seconds(&mut self) -> f64 { self.current_time }
    
    fn set_position_seconds(&mut self, pos: f64) {
        if let Some(stream) = self.ictx.stream(self.stream_index) {
            let tb = stream.time_base();
            let pts = (pos / (tb.numerator() as f64 / tb.denominator() as f64)) as i64;
            
            unsafe {
                ffmpeg_next::ffi::av_seek_frame(
                    self.ictx.as_mut_ptr(),
                    self.stream_index as i32,
                    pts,
                    ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD
                );
            }
        }
        self.decoder.flush();
        self.buf_pos = self.sample_buf.len();
        self.current_time = pos;
    }
    
    fn get_num_channels(&mut self) -> i32 { self.channels as i32 }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 { self.channel_vus.get(channel as usize).cloned().unwrap_or(0.0) }
    fn get_artist(&mut self) -> String { self.artist.clone() }
    fn get_type(&mut self) -> String { self.ext_type.clone() }
    fn get_tempo(&mut self) -> i32 { 0 }
    fn get_speed(&mut self) -> i32 { 0 }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> { Some(self.intrinsic_sample_rate) }
    fn get_num_samples(&mut self) -> i32 { 0 }
    fn get_num_instruments(&mut self) -> i32 { 0 }
    fn get_num_patterns(&mut self) -> i32 { 0 }
    fn get_current_order(&mut self) -> i32 { 0 }
    fn get_current_row(&mut self) -> i32 { 0 }

    fn get_video_info(&mut self) -> Option<String> { self.video_info.clone() }
    
    fn attach_video_queue(&mut self, tx: crossbeam_channel::Sender<ffmpeg_next::Packet>) {
        self.video_tx = Some(tx);
    }
    
    fn take_video_parameters(&mut self) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base.take()) {
            Some((p, tb))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------
// Video Only Source (No Audio)
// ---------------------------------------------------------
struct VideoOnlySource {
    ictx: ffmpeg_next::format::context::Input,
    video_stream_index: usize,
    video_tx: Option<crossbeam_channel::Sender<ffmpeg_next::Packet>>,
    video_params: Option<ffmpeg_next::codec::Parameters>,
    video_time_base: Option<ffmpeg_next::Rational>,
    current_time: f64,
    duration: f64,
    video_info: Option<String>,
    ext_type: String,
}

impl AudioSource for VideoOnlySource {
    fn read_frames(&mut self, hardware_channels: usize, sample_rate: u32, output: &mut [f32]) -> usize {
        let frames_to_write = output.len() / hardware_channels;
        for v in output.iter_mut() {
            *v = 0.0;
        }
        
        let mut packets_read = 0;
        while packets_read < 20 {
            if let Some((stream, packet)) = self.ictx.packets().next() {
                if stream.index() == self.video_stream_index {
                    if let Some(tx) = &self.video_tx {
                        let _ = tx.try_send(packet.clone());
                    }
                }
                packets_read += 1;
            } else {
                break;
            }
        }
        
        self.current_time += frames_to_write as f64 / sample_rate as f64;
        frames_to_write
    }
    
    fn get_duration_seconds(&mut self) -> f64 { self.duration }
    fn get_position_seconds(&mut self) -> f64 { self.current_time }
    
    fn set_position_seconds(&mut self, pos: f64) {
        if let Some(stream) = self.ictx.stream(self.video_stream_index) {
            let tb = stream.time_base();
            let pts = (pos / (tb.numerator() as f64 / tb.denominator() as f64)) as i64;
            unsafe {
                ffmpeg_next::ffi::av_seek_frame(
                    self.ictx.as_mut_ptr(),
                    self.video_stream_index as i32,
                    pts,
                    ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD
                );
            }
        }
        self.current_time = pos;
    }
    
    fn get_num_channels(&mut self) -> i32 { 2 }
    fn get_current_channel_vu_mono(&mut self, _channel: i32) -> f32 { 0.0 }
    fn get_artist(&mut self) -> String { "Video Only".to_string() }
    fn get_type(&mut self) -> String { self.ext_type.clone() }
    fn get_tempo(&mut self) -> i32 { 0 }
    fn get_speed(&mut self) -> i32 { 0 }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> { Some(44100) }
    fn get_num_samples(&mut self) -> i32 { 0 }
    fn get_num_instruments(&mut self) -> i32 { 0 }
    fn get_num_patterns(&mut self) -> i32 { 0 }
    fn get_current_order(&mut self) -> i32 { 0 }
    fn get_current_row(&mut self) -> i32 { 0 }
    fn get_video_info(&mut self) -> Option<String> { self.video_info.clone() }
    
    fn attach_video_queue(&mut self, tx: crossbeam_channel::Sender<ffmpeg_next::Packet>) {
        self.video_tx = Some(tx);
    }
    
    fn take_video_parameters(&mut self) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base.take()) {
            Some((p, tb))
        } else {
            None
        }
    }
}

fn try_ffmpeg(file_path: &str) -> Result<Box<dyn AudioSource>> {
    let _ = ffmpeg_next::init();
    let mut dict = ffmpeg_next::Dictionary::new();
    dict.set("probesize", "5000000");
    dict.set("analyzeduration", "5000000");
    let ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict).context("Failed to open file via libavformat")?;
    
    let duration = ictx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64;
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("FFMPEG")
        .to_uppercase();
        
    let mut video_info = None;
    let mut video_stream_index = None;
    let mut video_params = None;
    let mut video_tb = None;
    if let Some(v_stream) = ictx.streams().best(ffmpeg_next::media::Type::Video) {
        video_stream_index = Some(v_stream.index());
        video_params = Some(v_stream.parameters());
        video_tb = Some(v_stream.time_base());
        if let Ok(v_ctx) = ffmpeg_next::codec::context::Context::from_parameters(v_stream.parameters()) {
            if let Ok(v_dec) = v_ctx.decoder().video() {
                video_info = Some(format!("{} ({}x{})", v_dec.codec().map(|c| c.name().to_string()).unwrap_or("H264".to_string()).to_uppercase(), v_dec.width(), v_dec.height()));
            } else {
                video_info = Some("Unsupported Codec".to_string());
            }
        } else {
            video_info = Some("Unsupported Codec".to_string());
        }
    }

    let audio_stream = ictx.streams().best(ffmpeg_next::media::Type::Audio);
    
    if audio_stream.is_none() {
        if let Some(v_idx) = video_stream_index {
            return Ok(Box::new(VideoOnlySource {
                ictx,
                video_stream_index: v_idx,
                video_tx: None,
                video_params,
                video_time_base: video_tb,
                current_time: 0.0,
                duration,
                video_info,
                ext_type: ext,
            }));
        } else {
            return Err(anyhow::anyhow!("No audio stream found and no video stream found"));
        }
    }
    
    let stream = audio_stream.unwrap();
    let stream_index = stream.index();
    
    let context = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?;
    let decoder = context.decoder().audio()?;
    
    let channels = decoder.channels() as u16;
    let sample_rate = decoder.rate();
    let time_base = stream.time_base();
    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
    
    let resampler = ffmpeg_next::software::resampling::context::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
        decoder.channel_layout(),
        decoder.rate(),
    ).context("Failed to create resampler")?;

    Ok(Box::new(FfmpegSource {
        ictx,
        decoder,
        stream_index,
        resampler,
        sample_buf: Vec::new(),
        buf_pos: 0,
        channels,
        time_base: tb,
        current_time: 0.0,
        duration,
        artist: "Unknown".to_string(),
        ext_type: ext,
        intrinsic_sample_rate: sample_rate,
        video_info,
        channel_vus: vec![0.0; channels as usize],
        video_stream_index,
        video_tx: None,
        video_params,
        video_time_base: video_tb,
    }))
}


fn try_symphonia<R: symphonia::core::io::MediaSource + 'static>(file: R, probe_ext: &str, display_ext: &str, video_info: Option<String>) -> Result<Box<dyn AudioSource>> {
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(probe_ext);

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Unsupported audio format")?;

    let format = probed.format;
    
    // Find the first supported audio track instead of the default track (which might be video)
    let track = format.tracks().iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL && 
                  symphonia::default::get_codecs().make(&t.codec_params, &DecoderOptions::default()).is_ok())
        .context("No supported audio track found")?
        .clone();
        
    let track_id = track.id;
    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Unsupported codec")?;

    let sym_channels = track.codec_params.channels.unwrap_or(symphonia::core::audio::Channels::FRONT_LEFT | symphonia::core::audio::Channels::FRONT_RIGHT);
    let channels = sym_channels.count() as u16;
    let time_base = track.codec_params.time_base.map(|t| t.calc_time(1).seconds as f64 + t.calc_time(1).frac).unwrap_or(1.0 / 44100.0);
    let duration = track.codec_params.n_frames.map(|n| n as f64 * time_base).unwrap_or(0.0);

    let intrinsic_sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

    Ok(Box::new(SymphoniaSource {
        format,
        decoder,
        track_id,
        sample_buf: SampleBuffer::<f32>::new(0, symphonia::core::audio::SignalSpec::new(intrinsic_sample_rate, sym_channels)),
        buf_pos: 0,
        time_base,
        current_time: 0.0,
        duration,
        channels,
        channel_vus: vec![0.0; channels as usize],
        artist: "Unknown".to_string(),
        ext_type: if display_ext.is_empty() { "UNKNOWN".to_string() } else { display_ext.to_uppercase() },
        intrinsic_sample_rate: Some(intrinsic_sample_rate),
        video_info,
    }))
}

pub fn load_audio_source(file_path: &str) -> Result<Box<dyn AudioSource>> {
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
        
    let video_info = None; // Symphonia doesn't do video

    // 1. Prioritize FFmpeg for video containers (MKV, MP4) to ensure video streams are processed.
    // Symphonia might successfully parse the audio in these containers but would ignore the video.
    if ext == "mkv" || ext == "mp4" {
        if let Ok(source) = try_ffmpeg(file_path) {
            return Ok(source);
        }
        // Fallback to Symphonia if FFmpeg fails
        if let Ok(file) = File::open(file_path) {
            if let Ok(source) = try_symphonia(file, &ext, &ext, video_info.clone()) {
                return Ok(source);
            }
        }
    }

    // 2. Try standard audio formats natively via Symphonia first
    if ext == "wav" || ext == "flac" || ext == "mp3" || ext == "ogg" || ext == "m4a" || ext == "aac" {
        if let Ok(file) = File::open(file_path) {
            if let Ok(source) = try_symphonia(file, &ext, &ext, video_info.clone()) {
                return Ok(source);
            }
        }
    }

    // 2. Try OpenMPT Tracker module ONLY for likely tracker files first
    if ext == "mod" || ext == "s3m" || ext == "xm" || ext == "it" || ext == "mptm" {
        if let Ok(mut file) = File::open(file_path) {
            let mut data = Vec::new();
            if file.read_to_end(&mut data).is_ok() {
                let mut module_cursor = Cursor::new(data);
                if let Ok(module) = Module::create(&mut module_cursor, Logger::None, &[]) {
                    return Ok(Box::new(OpenMptSource { 
                        module: SafeModule(module),
                        left_buf: Vec::with_capacity(8192),
                        right_buf: Vec::with_capacity(8192),
                    }));
                }
            }
        }
    }

    // 3. Try MIDI
    if ext == "mid" || ext == "midi" {
        // Look for soundfont in project dir, then fallback
        let sf_path = if std::path::Path::new("assets/soundfont.sf2").exists() {
            "assets/soundfont.sf2"
        } else {
            "soundfont.sf2" // User must provide it in the working dir if not in assets
        };
        if let Ok(source) = MidiSource::new(file_path, sf_path, 48000) {
            return Ok(Box::new(source));
        } else {
            return Err(anyhow::anyhow!("Failed to parse MIDI or missing SoundFont ({}). Please place a SoundFont in assets/soundfont.sf2", sf_path));
        }
    }

    // 4. Try FFmpeg native bindings
    let ffmpeg_result = try_ffmpeg(file_path);
    if let Ok(source) = ffmpeg_result {
        return Ok(source);
    }

    // If all failed, return the ffmpeg error since it's the most descriptive for standard media
    ffmpeg_result
}

pub fn start_audio_thread(file_path: &str, mic: bool, shared_state: Arc<Mutex<AppState>>) -> Result<PlaybackHandle> {
    if !mic {
        let passthrough = shared_state.lock().unwrap().passthrough_enabled;
        if passthrough {
            let (tx, rx) = bounded::<DspMessage>(32);
            let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
            if let Ok((handle, decoder_rate)) = crate::bitstream::start_bitstream_thread(file_path, shared_state.clone(), tx.clone(), stop_token.clone()) {
                let max_frequency = shared_state.lock().unwrap().max_frequency;
                let sample_rate = decoder_rate;
                let window_size = (((sample_rate as f32 * 0.185).round() as usize) / 2) * 2;
                let window_size = window_size.max(2048).min(65536);
                
                {
                    let mut state = shared_state.lock().unwrap();
                    state.artist = "Bitstream Active".to_string();
                    state.module_type = "Hardware Passthrough".to_string();
                    state.stats.bitstream_active = true;
                    state.current_sample_rate = sample_rate as f32;
                    state.duration_seconds = 0.0;
                    state.num_channels = 8;
                    state.hardware_channels = 8;
                    state.channel_vus = vec![0.0; 8];
                    state.peak_vus = vec![0.0; 8];
                    state.video_info = Some("TrueHD/Atmos".to_string());
                }
                
                spawn_dsp_thread(rx, shared_state.clone(), sample_rate, max_frequency, window_size);
                return Ok(PlaybackHandle::Bitstream(handle, stop_token));
            }
        }
    }

    let host = cpal::default_host();
    let mut audio_source_opt = if mic { None } else { Some(load_audio_source(file_path)?) };
    let rate = audio_source_opt.as_mut().and_then(|a| a.get_intrinsic_sample_rate()).unwrap_or(48000);
    let target_rate: cpal::SampleRate = rate;
    let force_stereo = shared_state.lock().unwrap().force_stereo_downmix;
    let mut target_channels = audio_source_opt.as_mut().map(|a| a.get_num_channels() as u16).unwrap_or(2);
    if force_stereo {
        target_channels = 2;
    }

    let supported_config = if mic {
        let device = host.default_input_device().context("No input device available")?;
        let supported_configs_range = device.supported_input_configs().context("error while querying input configs")?;
        supported_configs_range
            .into_iter()
            .filter(|c| {
                c.max_sample_rate() >= target_rate
                    && c.min_sample_rate() <= target_rate
                    && (c.sample_format() == cpal::SampleFormat::F32
                        || c.sample_format() == cpal::SampleFormat::I16)
            })
            .next()
            .map(|c| c.with_sample_rate(target_rate))
            .or_else(|| device.default_input_config().ok())
            .context("No supported config?!")?
    } else {
        let device = host.default_output_device().context("No output device available")?;
        let supported_configs_range = device.supported_output_configs().context("error while querying output configs")?;
        
        let mut configs: Vec<_> = supported_configs_range
            .filter(|c| {
                c.sample_format() == cpal::SampleFormat::F32 || c.sample_format() == cpal::SampleFormat::I16
            })
            .collect();
            
        // Prefer exact channel match, else prefer stereo, then prefer rate match, then prefer most channels
        configs.sort_by_key(|c| {
            let ch_match = c.channels() == target_channels;
            let stereo = c.channels() >= 2;
            let rate_match = c.min_sample_rate() <= target_rate && c.max_sample_rate() >= target_rate;
            
            (
                std::cmp::Reverse(ch_match),
                std::cmp::Reverse(stereo),
                std::cmp::Reverse(rate_match),
                std::cmp::Reverse(c.channels())
            )
        });
        
        configs.into_iter()
            .next()
            .map(|c| {
                let rate = if c.min_sample_rate() <= target_rate && c.max_sample_rate() >= target_rate {
                    target_rate
                } else {
                    c.max_sample_rate()
                };
                c.with_sample_rate(rate)
            })
            .or_else(|| device.default_output_config().ok())
            .context("No supported config?!")?
    };

    let device = if mic {
        host.default_input_device().context("No input device available")?
    } else {
        host.default_output_device().context("No output device available")?
    };

    let config: cpal::StreamConfig = supported_config.clone().into();
    let (tx, rx) = bounded::<DspMessage>(32);

    if mic {
        {
            let mut state = shared_state.lock().unwrap();
            state.artist = "Microphone".to_string();
            state.module_type = "Hardware Input".to_string();
            state.duration_seconds = 0.0;
            state.num_channels = config.channels as i32;
            state.channel_vus = vec![0.0; state.num_channels as usize];
            state.bpm = 0;
            state.speed = 0;
            state.max_frequency = config.sample_rate as f32 / 2.0;
            state.current_sample_rate = config.sample_rate as f32;
        }

        let max_frequency = { shared_state.lock().unwrap().max_frequency };
        let window_size = (((config.sample_rate as f32 * 0.185).round() as usize) / 2) * 2;
        let window_size = window_size.max(2048).min(65536);
        spawn_dsp_thread(rx, shared_state.clone(), config.sample_rate, max_frequency, window_size);

        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => run_mic::<f32>(&device, &config, shared_state, tx, config.sample_rate),
            cpal::SampleFormat::I16 => run_mic::<i16>(&device, &config, shared_state, tx, config.sample_rate),
            cpal::SampleFormat::U16 => run_mic::<u16>(&device, &config, shared_state, tx, config.sample_rate),
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        }?;
        return Ok(PlaybackHandle::Cpal(stream));
    }

    let config: cpal::StreamConfig = supported_config.clone().into();
    let (tx, rx) = bounded::<DspMessage>(32);
    
    let mut audio_source = audio_source_opt.unwrap();
    
    {
        let mut state = shared_state.lock().unwrap();
        state.artist = audio_source.get_artist();
        state.module_type = audio_source.get_type();
        state.duration_seconds = audio_source.get_duration_seconds();
        let tracker_channels = audio_source.get_tracker_channels();
        state.num_channels = if let Some(tc) = tracker_channels {
            tc + 2
        } else {
            audio_source.get_num_channels()
        };
        state.hardware_channels = config.channels as i32;
        state.channel_vus = vec![0.0; state.num_channels as usize];
        state.peak_vus = vec![0.0; state.num_channels as usize];
        state.bpm = audio_source.get_tempo();
        state.speed = audio_source.get_speed();
        state.video_info = audio_source.get_video_info();
        state.max_frequency = audio_source.get_intrinsic_sample_rate()
            .map(|r| r as f32 / 2.0)
            .unwrap_or(10000.0);
        state.current_sample_rate = config.sample_rate as f32;
        state.num_samples = audio_source.get_num_samples();
        state.num_instruments = audio_source.get_num_instruments();
        state.num_patterns = audio_source.get_num_patterns();
        
        let _intrinsic = audio_source.get_num_channels();
        let _intrinsic = audio_source.get_num_channels();
        
        if !mic {
            state.tracker_channels = tracker_channels;
            state.tracker_patterns_by_order = audio_source.pre_format_tracker_data();
        }
    }

    let max_frequency = { shared_state.lock().unwrap().max_frequency };
    let window_size = (((config.sample_rate as f32 * 0.185).round() as usize) / 2) * 2;
    let window_size = window_size.max(2048).min(65536);
    
    spawn_dsp_thread(rx, shared_state.clone(), config.sample_rate, max_frequency, window_size);

    let stream = match supported_config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config, audio_source, shared_state, tx, config.sample_rate, max_frequency, window_size),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config, audio_source, shared_state, tx, config.sample_rate, max_frequency, window_size),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config, audio_source, shared_state, tx, config.sample_rate, max_frequency, window_size),
        _ => return Err(anyhow::anyhow!("Unsupported sample format")),
    }?;

    stream.play().context("Failed to play stream")?;

    Ok(PlaybackHandle::Cpal(stream))
}


struct AudioChunk {
    samples: Vec<f32>,
    valid_frames: usize,
    channel_vus: Vec<f32>,
    left_peak: f32,
    right_peak: f32,
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
    current_row_string: String,
    tracker_channels: Option<i32>,
    spatial_channels: i32,
    track_ended: bool,
}

fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut audio_source: Box<dyn AudioSource>,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    sample_rate: u32,
    _max_frequency: f32,
    window_size: usize,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample,
{
    let hardware_channels = config.channels as usize;
    
    let chunk_frames = 1024;
    let pool_size = 64; 
    
    let (ready_tx, ready_rx) = crossbeam_channel::bounded::<AudioChunk>(pool_size);
    let (free_tx, free_rx) = crossbeam_channel::bounded::<AudioChunk>(pool_size);
    
    let (video_packet_tx, video_packet_rx) = crossbeam_channel::bounded::<ffmpeg_next::Packet>(4096);
    audio_source.attach_video_queue(video_packet_tx);
    
    if let Some((params, time_base)) = audio_source.take_video_parameters() {
        let (video_frame_tx, video_frame_rx) = crossbeam_channel::bounded::<crate::state::VideoFrame>(16);
        let (free_video_frame_tx, free_video_frame_rx) = crossbeam_channel::bounded::<crate::state::VideoFrame>(16);
        
        for _ in 0..16 {
            let _ = free_video_frame_tx.try_send(crate::state::VideoFrame {
                pts: 0.0,
                width: 0,
                height: 0,
                y_plane: Vec::new(),
                u_plane: Vec::new(),
                v_plane: Vec::new(),
                y_stride: 0,
                u_stride: 0,
                v_stride: 0,
                bit_depth: 8,
                color_space: 0,
                color_range: 0,
            });
        }
        
        if let Ok(mut state) = shared_state.lock() {
            state.video_frame_rx = Some(video_frame_rx);
            state.free_video_frame_tx = Some(free_video_frame_tx.clone());
        }
        
        let state_for_video = shared_state.clone();
        let video_packet_rx_for_video = video_packet_rx.clone();
        std::thread::spawn(move || {
            if let Ok(context) = ffmpeg_next::codec::context::Context::from_parameters(params) {
                if let Ok(mut decoder) = context.decoder().video() {
                    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
                    let mut local_epoch = 0;
                    let mut fallback_pts_seconds = 0.0;
                    
                    while let Ok(packet) = video_packet_rx_for_video.recv() {
                        let mut track_ended = false;
                        if let Ok(state) = state_for_video.try_lock() {
                            track_ended = state.track_ended;
                            if state.seek_epoch > local_epoch {
                                decoder.flush();
                                local_epoch = state.seek_epoch;
                            }
                        }
                        
                        if track_ended { return; }
                        
                        if decoder.send_packet(&packet).is_ok() {
                            let mut decoded = ffmpeg_next::frame::Video::empty();
                            while decoder.receive_frame(&mut decoded).is_ok() {
                                let mut pts = decoded.timestamp().map(|t| t as f64 * tb)
                                    .or_else(|| decoded.pts().map(|p| p as f64 * tb))
                                    .unwrap_or(-1.0);
                                    
                                if pts < 0.0 {
                                    pts = fallback_pts_seconds;
                                    fallback_pts_seconds += 1.0 / 30.0;
                                } else {
                                    fallback_pts_seconds = pts + (1.0 / 30.0);
                                }
                                
                                loop {
                                    let (cached_seconds, current_epoch) = {
                                        if let Ok(state) = state_for_video.try_lock() {
                                            track_ended = state.track_ended;
                                            (state.current_seconds, state.seek_epoch)
                                        } else {
                                            continue;
                                        }
                                    };
                                    
                                    if track_ended || current_epoch > local_epoch {
                                        break;
                                    }
                                    
                                    if pts <= cached_seconds + 0.05 {
                                        break;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(2));
                                }
                                
                                if let Ok(mut frame) = free_video_frame_rx.recv() {
                                    frame.pts = pts;
                                    frame.width = decoded.width();
                                    frame.height = decoded.height();
                                    
                                    let format_name = format!("{:?}", decoded.format());
                                    frame.bit_depth = if format_name.contains("10LE") { 10 } else if format_name.contains("12LE") { 12 } else { 8 };
                                    frame.color_space = decoded.color_space() as u32;
                                    frame.color_range = decoded.color_range() as u32;
                                    
                                    frame.y_stride = decoded.stride(0);
                                    frame.u_stride = decoded.stride(1);
                                    frame.v_stride = decoded.stride(2);
                                    
                                    let height = decoded.height() as usize;
                                    let y_len = frame.y_stride * height;
                                    let u_len = frame.u_stride * (height / 2);
                                    let v_len = frame.v_stride * (height / 2);
                                    
                                    frame.y_plane.clear();
                                    let ptr_y = decoded.data(0).as_ptr();
                                    if !ptr_y.is_null() && y_len > 0 {
                                        frame.y_plane.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr_y, y_len) });
                                    }
                                    
                                    frame.u_plane.clear();
                                    let ptr_u = decoded.data(1).as_ptr();
                                    if !ptr_u.is_null() && u_len > 0 {
                                        frame.u_plane.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr_u, u_len) });
                                    }
                                    
                                    frame.v_plane.clear();
                                    let ptr_v = decoded.data(2).as_ptr();
                                    if !ptr_v.is_null() && v_len > 0 {
                                        frame.v_plane.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr_v, v_len) });
                                    }
                                    
                                    if let Err(crossbeam_channel::TrySendError::Full(f)) = video_frame_tx.try_send(frame) {
                                        let _ = free_video_frame_tx.try_send(f);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }
    
    for _ in 0..pool_size {
        let _ = free_tx.try_send(AudioChunk {
            samples: vec![0.0; chunk_frames * hardware_channels],
            valid_frames: 0,
            channel_vus: Vec::with_capacity(32),
            left_peak: 0.0,
            right_peak: 0.0,
            current_order: 0,
            current_row: 0,
            bpm: 0,
            speed: 0,
            current_seconds: 0.0,
            current_row_string: String::with_capacity(128),
            tracker_channels: None,
            spatial_channels: 0,
            track_ended: false,
        });
    }
    
    let state_for_decoder = shared_state.clone();
    let ready_rx_for_decoder = ready_rx.clone();
    let free_tx_for_decoder = free_tx.clone();
    let video_rx_for_decoder = video_packet_rx.clone();
    
    std::thread::spawn(move || {
        loop {
            if let Ok(mut state) = state_for_decoder.try_lock() {
                if let Some(pos) = state.seek_request.take() {
                    audio_source.set_position_seconds(pos);
                    state.seek_epoch += 1;
                    while let Ok(chunk) = ready_rx_for_decoder.try_recv() {
                        let _ = free_tx_for_decoder.try_send(chunk);
                    }
                }
            }
            
            let mut chunk = match free_rx.recv() {
                Ok(c) => c,
                Err(_) => break, // CPAL died
            };
            
            let decode_start = Instant::now();
            let frames_read = audio_source.read_frames(hardware_channels, sample_rate, &mut chunk.samples[..chunk_frames * hardware_channels]);
            let decode_elapsed = decode_start.elapsed().as_micros() as f32;
            
            chunk.valid_frames = frames_read;
            chunk.current_order = audio_source.get_current_order();
            chunk.current_row = audio_source.get_current_row();
            chunk.bpm = audio_source.get_tempo();
            chunk.speed = audio_source.get_speed();
            chunk.current_seconds = audio_source.get_position_seconds();
            chunk.current_row_string.clear();
            chunk.current_row_string.push_str(&audio_source.get_current_row_string());
            chunk.tracker_channels = audio_source.get_tracker_channels();
            chunk.spatial_channels = audio_source.get_num_channels();
            chunk.track_ended = frames_read == 0;
            
            let mut left_peak = 0.0_f32;
            let mut right_peak = 0.0_f32;
            let mut clips = 0;
            for i in 0..frames_read {
                let l_val = chunk.samples[i * hardware_channels];
                left_peak = left_peak.max(l_val.abs());
                if l_val.abs() >= 1.0 { clips += 1; }
                
                if hardware_channels > 1 {
                    let r_val = chunk.samples[i * hardware_channels + 1];
                    right_peak = right_peak.max(r_val.abs());
                    if r_val.abs() >= 1.0 { clips += 1; }
                } else {
                    right_peak = left_peak;
                }
            }
            chunk.left_peak = left_peak;
            chunk.right_peak = right_peak;
            
            let mut channel_vus = Vec::new();
            if let Some(num_mod_channels) = chunk.tracker_channels {
                channel_vus.push(left_peak);
                for i in 0..num_mod_channels {
                    channel_vus.push(audio_source.get_current_channel_vu_mono(i));
                }
                channel_vus.push(right_peak);
            } else {
                for i in 0..chunk.spatial_channels {
                    channel_vus.push(audio_source.get_current_channel_vu_mono(i));
                }
            }
            chunk.channel_vus.clear();
            chunk.channel_vus.extend_from_slice(&channel_vus);
            
            if let Ok(mut state) = state_for_decoder.try_lock() {
                state.stats.decode_us = state.stats.decode_us * 0.9 + decode_elapsed * 0.1;
                let fill_pct = (ready_rx_for_decoder.len() as f32 / pool_size as f32) * 100.0;
                state.stats.audio_buffer_fill_pct = fill_pct;
                let video_pct = (video_rx_for_decoder.len() as f32 / 256.0) * 100.0;
                state.stats.video_buffer_fill_pct = video_pct;
                state.stats.clipping_events += clips;
            }
            
            if ready_tx.send(chunk).is_err() {
                break;
            }
            
            if frames_read == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });

    let mut current_chunk: Option<AudioChunk> = None;
    let mut chunk_frame_pos = 0;
    
    let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
    let mut channel_fft_buffers: Vec<Vec<f32>> = vec![vec![0.0; window_size]; hardware_channels];
    let mut windowed_buffer: Vec<f32> = vec![0.0; window_size];
    let mut windowed_channels: Vec<Vec<f32>> = vec![vec![0.0; window_size]; hardware_channels];
    let mut spare_channels: Vec<Vec<f32>> = vec![vec![0.0; window_size]; hardware_channels];
    let mut fft_index = 0;
    
    let mut last_channel_vus = Vec::new();
    let mut last_current_order = 0;
    let mut last_current_row = 0;
    let mut last_bpm = 0;
    let mut last_speed = 0;
    let mut last_current_seconds = 0.0;
    let mut last_current_row_string = String::new();
    let mut was_paused = false;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            if let Ok(state) = shared_state.try_lock() {
                was_paused = state.is_paused;
            }
            
            if was_paused {
                for sample in data.iter_mut() {
                    *sample = T::from_sample(0.0);
                }
                if let Ok(mut state) = shared_state.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                return;
            }

            let frames_to_render = data.len() / hardware_channels;
            let mut frames_written = 0;
            
            while frames_written < frames_to_render {
                if current_chunk.is_none() {
                    match ready_rx.try_recv() {
                        Ok(chunk) => {
                            last_channel_vus.clear();
                            last_channel_vus.extend_from_slice(&chunk.channel_vus);
                            last_current_order = chunk.current_order;
                            last_current_row = chunk.current_row;
                            last_bpm = chunk.bpm;
                            last_speed = chunk.speed;
                            last_current_seconds = chunk.current_seconds;
                            last_current_row_string.clear();
                            last_current_row_string.push_str(&chunk.current_row_string);
                            
                            if chunk.track_ended {
                                if let Ok(mut state) = shared_state.try_lock() {
                                    state.track_ended = true;
                                }
                            }
                            
                            current_chunk = Some(chunk);
                            chunk_frame_pos = 0;
                        }
                        Err(_) => {
                            for frame in data[frames_written * hardware_channels..].chunks_mut(hardware_channels) {
                                for sample in frame.iter_mut() {
                                    *sample = T::from_sample(0.0);
                                }
                            }
                            break;
                        }
                    }
                }
                
                if let Some(chunk) = current_chunk.as_mut() {
                    let frames_available = chunk.valid_frames.saturating_sub(chunk_frame_pos);
                    let frames_needed = frames_to_render - frames_written;
                    
                    if frames_available == 0 {
                        let c = current_chunk.take().unwrap();
                        let _ = free_tx.try_send(c);
                        continue;
                    }
                    
                    let to_copy = std::cmp::min(frames_available, frames_needed);
                    
                    for i in 0..to_copy {
                        let src_frame_idx = chunk_frame_pos + i;
                        let dst_frame_idx = frames_written + i;
                        
                        let mut mono = 0.0;
                        for c in 0..hardware_channels {
                            let sample = chunk.samples[src_frame_idx * hardware_channels + c].clamp(-1.0, 1.0);
                            data[dst_frame_idx * hardware_channels + c] = T::from_sample(sample);
                            mono += sample;
                            channel_fft_buffers[c][fft_index] = sample;
                        }
                        mono /= hardware_channels as f32;
                        
                        fft_buffer[fft_index] = mono;
                        fft_index = (fft_index + 1) % window_size;
                    }
                    
                    chunk_frame_pos += to_copy;
                    frames_written += to_copy;
                }
            }

            // Re-use pre-allocated windowed_channels instead of heap-allocating per callback
            for i in 0..window_size {
                let idx = (fft_index + i) % window_size;
                windowed_buffer[i] = fft_buffer[idx];
                for c in 0..hardware_channels {
                    windowed_channels[c][i] = channel_fft_buffers[c][idx];
                }
            }
            
            // Swap filled buffers with spare set — zero allocations in the hot path
            std::mem::swap(&mut windowed_channels, &mut spare_channels);
            // spare_channels now has the filled data, windowed_channels has the empties
            
            let msg = DspMessage {
                audio_data: windowed_buffer.clone(),
                channel_vus: last_channel_vus.clone(),
                current_order: last_current_order,
                current_row: last_current_row,
                bpm: last_bpm,
                speed: last_speed,
                current_seconds: last_current_seconds,
                current_row_string: last_current_row_string.clone(),
                channel_audio_data: std::mem::replace(&mut spare_channels, vec![vec![0.0; window_size]; hardware_channels]),
            };
            
            let _ = tx.try_send(msg);
        },
        |err| eprintln!("an error occurred on stream: {}", err),
        None,
    )?;

    Ok(stream)
}

fn run_mic<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    _sample_rate: u32,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample + Into<f32>,
{
    let channels = config.channels as usize;
    let window_size = (((config.sample_rate as f32 * 0.185).round() as usize) / 2) * 2;
    let window_size = window_size.max(2048).min(65536);

    let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
    let mut channel_fft_buffers: Vec<Vec<f32>> = vec![vec![0.0; window_size]; channels];
    let mut windowed_buffer: Vec<f32> = vec![0.0; window_size];
    let mut fft_index = 0;
    
    let mut was_paused = false;

    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if let Ok(state) = shared_state.try_lock() {
                was_paused = state.is_paused;
            }
            
            if was_paused {
                if let Ok(mut state) = shared_state.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                }
                return;
            }

            let mut left_peak = 0.0_f32;
            let mut right_peak = 0.0_f32;

            for frame in data.chunks(channels) {
                let left = frame[0].into();
                let right = if channels >= 2 { frame[1].into() } else { left };
                
                left_peak = left_peak.max(left.abs());
                right_peak = right_peak.max(right.abs());

                let mono = (left + right) / 2.0;
                fft_buffer[fft_index] = mono;
                for c in 0..channels {
                    channel_fft_buffers[c][fft_index] = if c < frame.len() { frame[c].into() } else { 0.0 };
                }
                fft_index = (fft_index + 1) % window_size;
            }

            let mut windowed_channels = vec![vec![0.0; window_size]; channels];
            for i in 0..window_size {
                let idx = (fft_index + i) % window_size;
                windowed_buffer[i] = fft_buffer[idx];
                for c in 0..channels {
                    windowed_channels[c][i] = channel_fft_buffers[c][idx];
                }
            }
            
            let mut channel_vus = Vec::new();
            if channels >= 1 { channel_vus.push(left_peak); }
            if channels >= 2 { channel_vus.push(right_peak); }

            let msg = DspMessage {
                audio_data: windowed_buffer.clone(),
                channel_vus,
                current_order: 0,
                current_row: 0,
                bpm: 0,
                speed: 0,
                current_seconds: 0.0,
                current_row_string: String::new(),
                channel_audio_data: windowed_channels,
            };
            
            let _ = tx.try_send(msg);
        },
        |err| eprintln!("an error occurred on input stream: {}", err),
        None,
    )?;

    Ok(stream)
}
