use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openmpt::module::{Logger, Module};
use spectrum_analyzer::{samples_fft_to_spectrum, windows::hann_window, FrequencyLimit};
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

struct DspMessage {
    audio_data: Vec<f32>,
    channel_vus: Vec<f32>,
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
}

fn spawn_dsp_thread(
    rx: Receiver<DspMessage>,
    shared_state: Arc<Mutex<AppState>>,
    sample_rate: u32,
    max_frequency: f32,
) {
    std::thread::spawn(move || {
        let mut windowed_buffer = vec![0.0; 4096];
        let mut binned_data = vec![0.0; 512];

        while let Ok(msg) = rx.recv() {
            // Apply windowing and FFT
            for i in 0..4096 {
                windowed_buffer[i] = *msg.audio_data.get(i).unwrap_or(&0.0);
            }
            let fft_start = Instant::now();
            let hann = hann_window(&windowed_buffer);
            if let Ok(spectrum) = samples_fft_to_spectrum(
                &hann,
                sample_rate,
                FrequencyLimit::Max(max_frequency),
                Some(&spectrum_analyzer::scaling::divide_by_N_sqrt),
            ) {
                binned_data.fill(0.0);
                let bands: Vec<_> = spectrum.data().iter().collect();
                let step = std::cmp::max(1, (bands.len() as f32 / 512.0).ceil() as usize);
                
                for (i, chunk) in bands.chunks(step).enumerate() {
                    if i < 512 {
                        let max_val = chunk.iter().map(|(_, val)| val.val()).fold(0.0, f32::max);
                        binned_data[i] = (max_val * 150.0).clamp(0.0, 100.0);
                    }
                }
            }
            let fft_elapsed = fft_start.elapsed().as_micros() as f32;

            // Sync to UI state
            if let Ok(mut state) = shared_state.try_lock() {
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
                let search_limit = msg.audio_data.len().saturating_sub(512);
                for i in 0..search_limit {
                    if msg.audio_data[i] <= 0.0 && msg.audio_data[i + 1] > 0.0 {
                        let slope = msg.audio_data[i + 1] - msg.audio_data[i];
                        if slope > best_slope {
                            best_slope = slope;
                            start_idx = i;
                        }
                    }
                }
                
                for i in 0..512 {
                    state.raw_waveform[i] = msg.audio_data[start_idx + i];
                }
                
                state.waveform_history.pop_front();
                let wave_clone = state.raw_waveform.clone();
                state.waveform_history.push_back(wave_clone);
                
                // --- Fire Heat Decay ---
                for i in 0..512 {
                    let current = binned_data[i];
                    if current > state.fire_heat[i] {
                        state.fire_heat[i] = current; // Instant ignition
                    } else {
                        state.fire_heat[i] = (state.fire_heat[i] - 1.5).max(0.0); // Slow decay
                    }
                }
                
                if msg.bpm != 0 { state.bpm = msg.bpm; }
                if msg.speed != 0 { state.speed = msg.speed; }
                state.current_seconds = msg.current_seconds;
                
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
                
                state.spectrum_history.pop_front();
                state.spectrum_history.push_back(binned_data.to_vec());
            }
        }
    });
}

pub trait AudioSource: Send {
    fn read_float_stereo(&mut self, sample_rate: u32, left: &mut [f32], right: &mut [f32]) -> usize;
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
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>>;
}

// ---------------------------------------------------------
// OpenMPT Tracker Decoder
// ---------------------------------------------------------
struct SafeModule(Module);
unsafe impl Send for SafeModule {}

struct OpenMptSource {
    module: SafeModule,
}

impl AudioSource for OpenMptSource {
    fn read_float_stereo(&mut self, sample_rate: u32, left: &mut [f32], right: &mut [f32]) -> usize {
        let frames_to_render = left.len();
        let mut fake_left = unsafe { Vec::from_raw_parts(left.as_mut_ptr(), frames_to_render, frames_to_render) };
        let mut fake_right = unsafe { Vec::from_raw_parts(right.as_mut_ptr(), frames_to_render, frames_to_render) };

        let frames_read = self.module.0.read_float_stereo(sample_rate as i32, &mut fake_left, &mut fake_right);

        std::mem::forget(fake_left);
        std::mem::forget(fake_right);

        frames_read
    }

    fn get_duration_seconds(&mut self) -> f64 { self.module.0.get_duration_seconds() }
    fn get_position_seconds(&mut self) -> f64 { self.module.0.get_position_seconds() }
    fn set_position_seconds(&mut self, pos: f64) { self.module.0.set_position_seconds(pos); }
    fn get_num_channels(&mut self) -> i32 { self.module.0.get_num_channels() }
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
}

impl AudioSource for SymphoniaSource {
    fn read_float_stereo(&mut self, _sample_rate: u32, left: &mut [f32], right: &mut [f32]) -> usize {
        let mut frames_written = 0;
        let frames_needed = left.len();
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
                let left_idx = self.buf_pos + i * self.channels as usize;
                let right_idx = left_idx + if self.channels > 1 { 1 } else { 0 };
                
                left[frames_written + i] = samples[left_idx];
                right[frames_written + i] = samples[right_idx];

                let l_abs = samples[left_idx].abs();
                let r_abs = samples[right_idx].abs();
                self.channel_vus[0] = self.channel_vus[0].max(l_abs);
                if self.channels > 1 {
                    self.channel_vus[1] = self.channel_vus[1].max(r_abs);
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
        if let Ok(_) = self.format.seek(
            symphonia::core::formats::SeekMode::Accurate,
            symphonia::core::formats::SeekTo::Time {
                time: symphonia::core::units::Time::from(pos),
                track_id: Some(self.track_id),
            }
        ) {
            self.current_time = pos;
            self.buf_pos = self.sample_buf.len(); // drain buffer
        }
    }
    
    fn get_num_channels(&mut self) -> i32 { self.channels as i32 }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 { self.channel_vus.get(channel as usize).cloned().unwrap_or(0.0) * 100.0 }
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
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
}

fn load_audio_source(file_path: &str) -> Result<Box<dyn AudioSource>> {
    let mut file = File::open(file_path).context("Failed to open file")?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).context("Failed to read file")?;
    let cursor = Cursor::new(data.clone());

    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Try standard audio formats (WAV/FLAC) via Symphonia first
    if ext == "wav" || ext == "flac" || ext == "mp3" || ext == "ogg" {
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
        let mut hint = Hint::new();
        hint.with_extension(&ext);

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .context("Unsupported audio format")?;

        let format = probed.format;
        let track = format.default_track().context("No default track")?;
        let track_id = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .context("Unsupported codec")?;

        let channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);
        let time_base = track.codec_params.time_base.map(|t| t.calc_time(1).seconds as f64 + t.calc_time(1).frac).unwrap_or(1.0 / 44100.0);
        let duration = track.codec_params.n_frames.map(|n| n as f64 * time_base).unwrap_or(0.0);

        let intrinsic_sample_rate = track.codec_params.sample_rate;

        return Ok(Box::new(SymphoniaSource {
            format,
            decoder,
            track_id,
            sample_buf: SampleBuffer::<f32>::new(0, symphonia::core::audio::SignalSpec::new(0, symphonia::core::audio::Channels::empty())),
            buf_pos: 0,
            time_base,
            current_time: 0.0,
            duration,
            channels,
            channel_vus: vec![0.0; channels as usize],
            artist: "Unknown".to_string(),
            ext_type: ext.to_uppercase(),
            intrinsic_sample_rate,
        }));
    }

    // Fallback to OpenMPT Tracker module
    let mut module_cursor = Cursor::new(data);
    let mut module = Module::create(&mut module_cursor, Logger::None, &[])
        .map_err(|_| anyhow::anyhow!("Failed to create module"))?;
    module.set_repeat_count(0);
        
    Ok(Box::new(OpenMptSource { module: SafeModule(module) }))
}

pub fn start_audio_thread(file_path: &str, mic: bool, shared_state: Arc<Mutex<AppState>>) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let target_rate: cpal::SampleRate = 48000;

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

    let mut audio_source = load_audio_source(file_path)?;

    {
        let mut state = shared_state.lock().unwrap();
        state.artist = audio_source.get_artist();
        state.module_type = audio_source.get_type();
        state.duration_seconds = audio_source.get_duration_seconds();
        state.num_channels = audio_source.get_num_channels();
        state.channel_vus = vec![0.0; state.num_channels as usize];
        state.bpm = audio_source.get_tempo();
        state.speed = audio_source.get_speed();
        state.max_frequency = audio_source.get_intrinsic_sample_rate()
            .map(|r| r as f32 / 2.0)
            .unwrap_or(10000.0);
        state.num_samples = audio_source.get_num_samples();
        state.num_instruments = audio_source.get_num_instruments();
        state.num_patterns = audio_source.get_num_patterns();
        
        let formatted = audio_source.pre_format_tracker_data();
        state.tracker_patterns_by_order = formatted;
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
    let channels = config.channels as usize;
    let mut left_buffer: Vec<f32> = vec![0.0; 8192];
    let mut right_buffer: Vec<f32> = vec![0.0; 8192];
    
    let mut fft_buffer: Vec<f32> = vec![0.0; 4096];
    let mut windowed_buffer: Vec<f32> = vec![0.0; 4096];
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

            let frames_to_render = data.len() / channels;

            if left_buffer.capacity() < frames_to_render {
                left_buffer.reserve(frames_to_render - left_buffer.len());
            }
            if right_buffer.capacity() < frames_to_render {
                right_buffer.reserve(frames_to_render - right_buffer.len());
            }

            let mut fake_left = unsafe { Vec::from_raw_parts(left_buffer.as_mut_ptr(), frames_to_render, frames_to_render) };
            let mut fake_right = unsafe { Vec::from_raw_parts(right_buffer.as_mut_ptr(), frames_to_render, frames_to_render) };

            let decode_start = Instant::now();
            let frames_read = audio_source.read_float_stereo(
                sample_rate,
                &mut fake_left,
                &mut fake_right,
            );
            let decode_elapsed = decode_start.elapsed().as_micros() as f32;

            std::mem::forget(fake_left);
            std::mem::forget(fake_right);
            
            if frames_read == 0 {
                if let Ok(mut state) = shared_state.try_lock() {
                    state.track_ended = true;
                }
            }

            for (i, frame) in data.chunks_mut(channels).enumerate() {
                if i < frames_read {
                    let left = left_buffer[i].clamp(-1.0, 1.0);
                    let right = right_buffer[i].clamp(-1.0, 1.0);
                    
                    let mono = (left + right) / 2.0;
                    fft_buffer[fft_index] = mono;
                    fft_index = (fft_index + 1) % 4096;

                    if channels >= 2 {
                        frame[0] = T::from_sample(left);
                        frame[1] = T::from_sample(right);
                        for sample in frame.iter_mut().skip(2) {
                            *sample = T::from_sample(0.0);
                        }
                    } else {
                        frame[0] = T::from_sample(mono);
                        for sample in frame.iter_mut().skip(1) {
                            *sample = T::from_sample(0.0);
                        }
                    }
                } else {
                    for sample in frame.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                }
            }

            for i in 0..4096 {
                windowed_buffer[i] = fft_buffer[(fft_index + i) % 4096];
            }
            
            let mut channel_vus = Vec::new();
            let num_mod_channels = audio_source.get_num_channels();
            for i in 0..num_mod_channels {
                channel_vus.push(audio_source.get_current_channel_vu_mono(i));
            }

            let msg = DspMessage {
                audio_data: windowed_buffer.clone(),
                channel_vus,
                current_order: audio_source.get_current_order(),
                current_row: audio_source.get_current_row(),
                bpm: audio_source.get_tempo(),
                speed: audio_source.get_speed(),
                current_seconds: audio_source.get_position_seconds(),
            };
            
            let _ = tx.try_send(msg);

            // Zero-allocation UI state sync via lock-free try_lock
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
    let mut fft_buffer: Vec<f32> = vec![0.0; 4096];
    let mut windowed_buffer: Vec<f32> = vec![0.0; 4096];
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
                fft_index = (fft_index + 1) % 4096;
            }

            for i in 0..4096 {
                windowed_buffer[i] = fft_buffer[(fft_index + i) % 4096];
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
            };
            
            let _ = tx.try_send(msg);
        },
        |err| eprintln!("an error occurred on input stream: {}", err),
        None,
    )?;

    Ok(stream)
}
