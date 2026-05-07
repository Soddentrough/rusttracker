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

use crate::state::AppState;
use crossbeam_channel::{bounded, Sender, Receiver};

const FFT_SIZE: usize = 8192;

struct DspMessage {
    audio_data: Vec<f32>,
    channel_vus: Vec<f32>,
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
    current_row_string: String,
}

fn spawn_dsp_thread(
    rx: Receiver<DspMessage>,
    shared_state: Arc<Mutex<AppState>>,
    sample_rate: u32,
    max_frequency: f32,
) {
    std::thread::spawn(move || {
        let mut binned_data = vec![0.0; 1024];

        // Pre-compute Hann window coefficients
        let hann_window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos()))
            .collect();

        // Pre-plan FFT (rustfft caches the twiddle factors)
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let mut complex_buf = vec![Complex { re: 0.0f32, im: 0.0f32 }; FFT_SIZE];
        let mut magnitudes = vec![0.0f32; FFT_SIZE / 2];

        while let Ok(msg) = rx.recv() {
            let fft_start = Instant::now();

            // Apply Hann window and prepare complex input
            for i in 0..FFT_SIZE {
                let sample = *msg.audio_data.get(i).unwrap_or(&0.0);
                complex_buf[i] = Complex { re: sample * hann_window[i], im: 0.0 };
            }

            // Run FFT
            fft.process(&mut complex_buf);

            // Compute magnitudes (only first half — up to Nyquist)
            let n_sqrt = (FFT_SIZE as f32).sqrt();
            for i in 0..FFT_SIZE / 2 {
                magnitudes[i] = complex_buf[i].norm() / n_sqrt;
            }

            // Logarithmic binning into 1024 display bins
            let resolution = sample_rate as f32 / FFT_SIZE as f32; // Hz per FFT bin
            let min_freq = 20.0f32;
            let max_f = max_frequency.max(min_freq * 2.0);
            let num_bins = 1024;
            let nyquist = FFT_SIZE / 2;

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
                
                state.raw_channel_vus.clear();
                for vu in msg.channel_vus {
                    state.raw_channel_vus.push(vu);
                }
                
                state.raw_spectrum_data.copy_from_slice(&binned_data);
                
                // --- Waveform extraction (Zero-Crossing Edge Trigger) ---
                // Scan the first half of the buffer for the steepest positive zero-crossing.
                // This acts as a heuristic to lock onto the fundamental frequency (bass/kick)
                // rather than triggering randomly on high-frequency noise ripples.
                let mut start_idx = 0;
                let mut best_slope = 0.0;
                let search_limit = msg.audio_data.len().saturating_sub(1024);
                for i in 0..search_limit {
                    if msg.audio_data[i] <= 0.0 && msg.audio_data[i + 1] > 0.0 {
                        let slope = msg.audio_data[i + 1] - msg.audio_data[i];
                        if slope > best_slope {
                            best_slope = slope;
                            start_idx = i;
                        }
                    }
                }
                
                for i in 0..1024 {
                    state.raw_waveform[i] = msg.audio_data[start_idx + i];
                }
                
                state.waveform_history.pop_front();
                let wave_clone = state.raw_waveform.clone();
                state.waveform_history.push_back(wave_clone);
                

                
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
}

impl FfmpegSource {
    fn get_next_frame(&mut self) -> bool {
        let mut decoded = ffmpeg_next::frame::Audio::empty();
        
        // 1. Try to receive a frame from already-buffered packets
        if self.decoder.receive_frame(&mut decoded).is_ok() {
            let mut resampled = ffmpeg_next::frame::Audio::empty();
            if self.resampler.run(&decoded, &mut resampled).is_ok() {
                // ffmpeg-next has a bug where plane<T>(0) for Packed format only returns length = samples(), 
                // instead of samples() * channels(). We must manually extract the full interleaved slice.
                let data = resampled.plane::<f32>(0);
                let actual_len = resampled.samples() * resampled.channels() as usize;
                let actual_data = unsafe { std::slice::from_raw_parts(data.as_ptr(), actual_len) };
                
                self.sample_buf.clear();
                self.sample_buf.extend_from_slice(actual_data);
                self.buf_pos = 0;
                return true;
            }
        }
        
        // 2. Decoder needs more data, read packets
        for (stream, packet) in self.ictx.packets() {
            if stream.index() == self.stream_index {
                // Send the packet to the decoder
                if let Err(_e) = self.decoder.send_packet(&packet) {
                    // Packet might be rejected if decoder is full, but we just drained it above.
                    // Or it could be a decode error. We continue to see if we can receive.
                }
                
                // Now try to receive a frame
                if self.decoder.receive_frame(&mut decoded).is_ok() {
                    let mut resampled = ffmpeg_next::frame::Audio::empty();
                    if self.resampler.run(&decoded, &mut resampled).is_ok() {
                        // ffmpeg-next bug workaround for Packed formats
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
                    
                    output[out_idx] = (l + c * 0.707 + l_surround * 0.707).clamp(-1.0, 1.0);
                    output[out_idx + 1] = (r + c * 0.707 + r_surround * 0.707).clamp(-1.0, 1.0);
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
}

fn try_ffmpeg(file_path: &str) -> Result<Box<dyn AudioSource>> {
    let _ = ffmpeg_next::init();
    let ictx = ffmpeg_next::format::input(&file_path).context("Failed to open file via libavformat")?;
    
    let stream = ictx.streams().best(ffmpeg_next::media::Type::Audio).context("No audio stream found")?;
    let stream_index = stream.index();
    
    let context = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?;
    let decoder = context.decoder().audio()?;
    
    let channels = decoder.channels() as u16;
    let sample_rate = decoder.rate();
    let time_base = stream.time_base();
    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
    let duration = ictx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64;
    
    let mut video_info = None;
    if let Some(v_stream) = ictx.streams().best(ffmpeg_next::media::Type::Video) {
        let v_ctx = ffmpeg_next::codec::context::Context::from_parameters(v_stream.parameters())?;
        if let Ok(v_dec) = v_ctx.decoder().video() {
            video_info = Some(format!("{} ({}x{})", v_dec.codec().map(|c| c.name().to_string()).unwrap_or("H264".to_string()).to_uppercase(), v_dec.width(), v_dec.height()));
        }
    }
    
    let resampler = ffmpeg_next::software::resampling::context::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
        decoder.channel_layout(),
        decoder.rate(),
    ).context("Failed to create resampler")?;

    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("FFMPEG")
        .to_uppercase();

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

    // 1. Try standard audio formats natively via Symphonia first
    if ext == "wav" || ext == "flac" || ext == "mp3" || ext == "ogg" || ext == "mp4" || ext == "m4a" || ext == "aac" || ext == "mkv" {
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

    // 3. Try FFmpeg native bindings
    let ffmpeg_result = try_ffmpeg(file_path);
    if let Ok(source) = ffmpeg_result {
        return Ok(source);
    }

    // If all failed, return the ffmpeg error since it's the most descriptive for standard media
    ffmpeg_result
}

pub fn start_audio_thread(file_path: &str, mic: bool, shared_state: Arc<Mutex<AppState>>) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let mut audio_source_opt = if mic { None } else { Some(load_audio_source(file_path)?) };
    let rate = audio_source_opt.as_mut().and_then(|a| a.get_intrinsic_sample_rate()).unwrap_or(48000);
    let target_rate: cpal::SampleRate = rate;
    let target_channels = audio_source_opt.as_mut().map(|a| a.get_num_channels() as u16).unwrap_or(2);

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
            
        // Prefer exact channel match, else prefer stereo, then prefer rate match
        configs.sort_by(|a, b| {
            let a_ch_match = a.channels() == target_channels;
            let b_ch_match = b.channels() == target_channels;
            if a_ch_match != b_ch_match { return b_ch_match.cmp(&a_ch_match); }
            
            let a_stereo = a.channels() >= 2;
            let b_stereo = b.channels() >= 2;
            if a_stereo != b_stereo { return b_stereo.cmp(&a_stereo); }
            
            let a_rate_match = a.min_sample_rate() <= target_rate && a.max_sample_rate() >= target_rate;
            let b_rate_match = b.min_sample_rate() <= target_rate && b.max_sample_rate() >= target_rate;
            if a_rate_match != b_rate_match { return b_rate_match.cmp(&a_rate_match); }
            
            b.channels().cmp(&a.channels())
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
        }

        let max_frequency = { shared_state.lock().unwrap().max_frequency };
        spawn_dsp_thread(rx, shared_state.clone(), config.sample_rate, max_frequency);

        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => run_mic::<f32>(&device, &config, shared_state, tx, config.sample_rate),
            cpal::SampleFormat::I16 => run_mic::<i16>(&device, &config, shared_state, tx, config.sample_rate),
            cpal::SampleFormat::U16 => run_mic::<u16>(&device, &config, shared_state, tx, config.sample_rate),
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        }?;
        return Ok(stream);
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
        state.channel_vus = vec![0.0; state.num_channels as usize];
        state.peak_vus = vec![0.0; state.num_channels as usize];
        state.bpm = audio_source.get_tempo();
        state.speed = audio_source.get_speed();
        state.video_info = audio_source.get_video_info();
        state.max_frequency = audio_source.get_intrinsic_sample_rate()
            .map(|r| r as f32 / 2.0)
            .unwrap_or(10000.0);
        state.num_samples = audio_source.get_num_samples();
        state.num_instruments = audio_source.get_num_instruments();
        state.num_patterns = audio_source.get_num_patterns();
        
        let intrinsic = audio_source.get_num_channels();
        if intrinsic > 2 {
            state.available_visualizers = vec![0, 4, 5, 3, 1, 2];
        } else {
            state.available_visualizers = vec![0, 1, 2];
        }
        if !mic {
            state.tracker_channels = tracker_channels;
            state.tracker_patterns_by_order = audio_source.pre_format_tracker_data();
        }
    }

    let max_frequency = { shared_state.lock().unwrap().max_frequency };
    spawn_dsp_thread(rx, shared_state.clone(), config.sample_rate, max_frequency);

    let stream = match supported_config.sample_format() {
        cpal::SampleFormat::F32 => run::<f32>(&device, &config, audio_source, shared_state, tx, config.sample_rate, max_frequency),
        cpal::SampleFormat::I16 => run::<i16>(&device, &config, audio_source, shared_state, tx, config.sample_rate, max_frequency),
        cpal::SampleFormat::U16 => run::<u16>(&device, &config, audio_source, shared_state, tx, config.sample_rate, max_frequency),
        _ => return Err(anyhow::anyhow!("Unsupported sample format")),
    }?;

    stream.play().context("Failed to play stream")?;

    Ok(stream)
}

fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut audio_source: Box<dyn AudioSource>,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    sample_rate: u32,
    _max_frequency: f32,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample,
{
    let hardware_channels = config.channels as usize;
    let mut interleaved_buffer: Vec<f32> = vec![0.0; 8192 * hardware_channels];
    
    let mut fft_buffer: Vec<f32> = vec![0.0; 8192];
    let mut windowed_buffer: Vec<f32> = vec![0.0; 8192];
    let mut fft_index = 0;
    
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

            let needed_len = frames_to_render * hardware_channels;
            if interleaved_buffer.len() < needed_len {
                interleaved_buffer.resize(needed_len, 0.0);
            }

            let decode_start = Instant::now();
            let frames_read = audio_source.read_frames(
                hardware_channels,
                sample_rate,
                &mut interleaved_buffer[..needed_len],
            );
            let decode_elapsed = decode_start.elapsed().as_micros() as f32;
            
            if frames_read == 0 {
                if let Ok(mut state) = shared_state.try_lock() {
                    state.track_ended = true;
                }
            }

            let mut left_peak = 0.0_f32;
            let mut right_peak = 0.0_f32;

            for (i, frame) in data.chunks_mut(hardware_channels).enumerate() {
                if i < frames_read {
                    let mut mono = 0.0;
                    for c in 0..hardware_channels {
                        let sample = interleaved_buffer[i * hardware_channels + c].clamp(-1.0, 1.0);
                        frame[c] = T::from_sample(sample);
                        mono += sample;
                        
                        if c == 0 {
                            left_peak = left_peak.max(sample.abs());
                        } else if c == 1 {
                            right_peak = right_peak.max(sample.abs());
                        }
                    }
                    if hardware_channels == 1 {
                        right_peak = left_peak; // Mono fallback
                    }
                    mono /= hardware_channels as f32;
                    
                    fft_buffer[fft_index] = mono;
                    fft_index = (fft_index + 1) % 8192;
                } else {
                    for sample in frame.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                }
            }

            for i in 0..8192 {
                windowed_buffer[i] = fft_buffer[(fft_index + i) % 8192];
            }
            
            let mut channel_vus = Vec::new();
            if let Some(num_mod_channels) = audio_source.get_tracker_channels() {
                // Tracker module layout: [L, Track1, Track2, ..., TrackN, R]
                channel_vus.push(left_peak);
                for i in 0..num_mod_channels {
                    channel_vus.push(audio_source.get_current_channel_vu_mono(i));
                }
                channel_vus.push(right_peak);
            } else {
                // Media file layout: [FL, FR, FC, LFE, ...]
                let num_spatial_channels = audio_source.get_num_channels();
                for i in 0..num_spatial_channels {
                    channel_vus.push(audio_source.get_current_channel_vu_mono(i));
                }
            }

            let msg = DspMessage {
                audio_data: windowed_buffer.clone(),
                channel_vus,
                current_order: audio_source.get_current_order(),
                current_row: audio_source.get_current_row(),
                bpm: audio_source.get_tempo(),
                speed: audio_source.get_speed(),
                current_seconds: audio_source.get_position_seconds(),
                current_row_string: audio_source.get_current_row_string(),
            };
            
            let _ = tx.try_send(msg);

            if let Ok(mut state) = shared_state.try_lock() {
                state.stats.decode_us = state.stats.decode_us * 0.9 + decode_elapsed * 0.1;
                if let Some(pos) = state.seek_request.take() {
                    audio_source.set_position_seconds(pos);
                }
            }
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
    let mut fft_buffer: Vec<f32> = vec![0.0; 8192];
    let mut windowed_buffer: Vec<f32> = vec![0.0; 8192];
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
                fft_index = (fft_index + 1) % 8192;
            }

            for i in 0..8192 {
                windowed_buffer[i] = fft_buffer[(fft_index + i) % 8192];
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
            };
            
            let _ = tx.try_send(msg);
        },
        |err| eprintln!("an error occurred on input stream: {}", err),
        None,
    )?;

    Ok(stream)
}
