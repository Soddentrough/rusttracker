use ffmpeg_next as ffmpeg;
use crossbeam_channel::{bounded, Sender, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Instant, Duration};

struct MockAppState {
    pub current_seconds: f64,
    pub seek_epoch: u64,
    pub track_ended: bool,
    pub video_frame_rx: Option<Receiver<VideoFrame>>,
    pub free_video_frame_tx: Option<Sender<VideoFrame>>,
    pub seek_request: Option<f64>,
}

#[derive(Debug, Clone)]
struct VideoFrame {
    pts: f64,
    width: u32,
    height: u32,
}

pub trait AudioSource: Send {
    fn read_frames(&mut self, hardware_channels: usize, sample_rate: u32, output: &mut [f32]) -> usize;
    fn attach_video_queue(&mut self, tx: Sender<(u64, ffmpeg::Packet)>);
    fn take_video_parameters(&mut self) -> Option<(ffmpeg::codec::Parameters, ffmpeg::Rational)>;
    fn set_position_seconds(&mut self, pos: f64);
}

struct FfmpegSource {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Audio,
    stream_index: usize,
    resampler: ffmpeg::software::resampling::Context,
    
    sample_buf: Vec<f32>,
    buf_pos: usize,
    
    channels: u16,
    time_base: f64,
    current_time: f64,
    video_stream_index: Option<usize>,
    video_tx: Option<Sender<(u64, ffmpeg::Packet)>>,
    video_epoch: u64,
    video_params: Option<ffmpeg::codec::Parameters>,
    video_time_base: Option<ffmpeg::Rational>,
    packets_after_seek: usize,
}

impl FfmpegSource {
    fn get_next_frame(&mut self) -> bool {
        let mut decoded = ffmpeg::frame::Audio::empty();
        
        if self.decoder.receive_frame(&mut decoded).is_ok() {
            let mut resampled = ffmpeg::frame::Audio::empty();
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
                Err(_) => {}
            }
        }
        
        for (stream, packet) in self.ictx.packets() {
            if self.packets_after_seek < 20 {
                let stream_type = if Some(stream.index()) == self.video_stream_index {
                    "Video"
                } else if stream.index() == self.stream_index {
                    "Audio"
                } else {
                    "Other"
                };
                println!(
                    "[Audio Source post-seek packet {}] Type={}, StreamIndex={}, PTS={:?}, DTS={:?}, Key={}",
                    self.packets_after_seek,
                    stream_type,
                    stream.index(),
                    packet.pts(),
                    packet.dts(),
                    packet.is_key()
                );
                self.packets_after_seek += 1;
            }
            if Some(stream.index()) == self.video_stream_index {
                if let Some(tx) = &self.video_tx {
                    let _ = tx.try_send((self.video_epoch, packet.clone()));
                }
            } else if stream.index() == self.stream_index {
                let _ = self.decoder.send_packet(&packet);
                
                if self.decoder.receive_frame(&mut decoded).is_ok() {
                    let mut resampled = ffmpeg::frame::Audio::empty();
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
                        Err(_) => {}
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

        while frames_written < frames_needed {
            let frames_available = (self.sample_buf.len() - self.buf_pos) / self.channels as usize;
            
            if frames_available == 0 {
                if !self.get_next_frame() {
                    break;
                }
                continue;
            }

            let frames_to_copy = std::cmp::min(frames_needed - frames_written, frames_available);
            self.buf_pos += frames_to_copy * self.channels as usize;
            frames_written += frames_to_copy;
        }

        frames_written
    }
    
    fn attach_video_queue(&mut self, tx: Sender<(u64, ffmpeg::Packet)>) {
        self.video_tx = Some(tx);
    }
    
    fn take_video_parameters(&mut self) -> Option<(ffmpeg::codec::Parameters, ffmpeg::Rational)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base.take()) {
            Some((p, tb))
        } else {
            None
        }
    }
    
    fn set_position_seconds(&mut self, pos: f64) {
        println!("[Audio Source] Global Seeking to {:.3}s (index -1)", pos);
        let seek_start = Instant::now();
        let pts = (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;
        let ret = unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                -1,
                pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD
            )
        };
        println!(
            "[Audio Source] Global av_seek_frame returned {} (elapsed: {:?})",
            ret,
            seek_start.elapsed()
        );
        self.decoder.flush();
        self.buf_pos = self.sample_buf.len();
        self.current_time = pos;
        self.video_epoch += 1;
        self.packets_after_seek = 0;
    }
}

fn try_ffmpeg(file_path: &str) -> Result<Box<dyn AudioSource>, Box<dyn std::error::Error>> {
    ffmpeg::init()?;
    let mut dict = ffmpeg::Dictionary::new();
    dict.set("probesize", "5000000");
    dict.set("analyzeduration", "5000000");
    let ictx = ffmpeg::format::input_with_dictionary(&file_path, dict)?;
    
    let mut video_stream_index = None;
    let mut video_params = None;
    let mut video_tb = None;
    if let Some(v_stream) = ictx.streams().best(ffmpeg::media::Type::Video) {
        video_stream_index = Some(v_stream.index());
        video_params = Some(v_stream.parameters());
        video_tb = Some(v_stream.time_base());
    }

    let audio_stream = ictx.streams().best(ffmpeg::media::Type::Audio).ok_or("No audio stream")?;
    let stream_index = audio_stream.index();
    
    let context = ffmpeg::codec::context::Context::from_parameters(audio_stream.parameters())?;
    let decoder = context.decoder().audio()?;
    
    let channels = decoder.channels() as u16;
    let time_base = audio_stream.time_base();
    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
    
    let resampler = ffmpeg::software::resampling::context::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg::format::sample::Sample::F32(ffmpeg::format::sample::Type::Packed),
        decoder.channel_layout(),
        decoder.rate(),
    )?;

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
        video_stream_index,
        video_tx: None,
        video_epoch: 0,
        video_params,
        video_time_base: video_tb,
        packets_after_seek: 9999, // default to high so it doesn't log on start
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file_path = "/home/naoki/Love.Death.and.Robots.S04E01.1080p.WEB.h264-ETHEL[EZTVx.to].mkv";
    println!("Opening source: {}", file_path);
    let mut audio_source = try_ffmpeg(file_path)?;
    
    let shared_state = Arc::new(Mutex::new(MockAppState {
        current_seconds: 0.0,
        seek_epoch: 0,
        track_ended: false,
        video_frame_rx: None,
        free_video_frame_tx: None,
        seek_request: None,
    }));
    
    let (video_packet_tx, video_packet_rx) = bounded::<(u64, ffmpeg::Packet)>(4096);
    audio_source.attach_video_queue(video_packet_tx);
    
    if let Some((params, time_base)) = audio_source.take_video_parameters() {
        println!("Video stream parameters found. Spawning video thread.");
        let (video_frame_tx, video_frame_rx) = bounded::<VideoFrame>(16);
        let (free_video_frame_tx, free_video_frame_rx) = bounded::<VideoFrame>(16);
        
        for _ in 0..16 {
            let _ = free_video_frame_tx.try_send(VideoFrame {
                pts: 0.0,
                width: 0,
                height: 0,
            });
        }
        
        {
            let mut state = shared_state.lock().unwrap();
            state.video_frame_rx = Some(video_frame_rx);
            state.free_video_frame_tx = Some(free_video_frame_tx.clone());
        }
        
        let state_for_video = shared_state.clone();
        let video_packet_rx_for_video = video_packet_rx.clone();
        std::thread::spawn(move || {
            println!("[Video Thread] Spawned");
            if let Ok(context) = ffmpeg::codec::context::Context::from_parameters(params) {
                if let Ok(mut decoder) = context.decoder().video() {
                    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
                    let mut local_epoch = 0;
                    let mut fallback_pts_seconds = 0.0;
                    let mut is_first_frame_after_seek = false;
                    
                    while let Ok((packet_epoch, packet)) = video_packet_rx_for_video.recv() {
                        {
                            let state = state_for_video.lock().unwrap();
                            if state.seek_epoch > local_epoch {
                                println!("[Video Thread] Epoch change: state.seek_epoch={} > local_epoch={}. Flushing decoder.", state.seek_epoch, local_epoch);
                                decoder.flush();
                                local_epoch = state.seek_epoch;
                                is_first_frame_after_seek = true;
                            }
                        }
                        
                        if packet_epoch < local_epoch {
                            continue;
                        }
                        
                        if let Err(e) = decoder.send_packet(&packet) {
                            println!("[Video Thread] send_packet error: {:?}", e);
                        } else {
                            let mut decoded = ffmpeg::frame::Video::empty();
                            loop {
                                match decoder.receive_frame(&mut decoded) {
                                    Ok(_) => {
                                        let mut pts = decoded.timestamp().map(|t| t as f64 * tb)
                                            .or_else(|| decoded.pts().map(|p| p as f64 * tb))
                                            .unwrap_or(-1.0);
                                            
                                        if pts < 0.0 {
                                            pts = fallback_pts_seconds;
                                            fallback_pts_seconds += 1.0 / 30.0;
                                        } else {
                                            fallback_pts_seconds = pts + (1.0 / 30.0);
                                        }
                                        
                                        println!("[Video Thread] Decoded frame at PTS: {:.3}", pts);
                                        
                                        let mut skip_push = false;
                                        let mut cached_seconds = 0.0;
                                        if is_first_frame_after_seek {
                                            println!("[Video Thread] First frame after seek detected. Pushing frame immediately: PTS={:.3}", pts);
                                            is_first_frame_after_seek = false;
                                        } else {
                                            let sync_start = Instant::now();
                                            loop {
                                                let (cs, ce, track_ended) = {
                                                    let state = state_for_video.lock().unwrap();
                                                    (state.current_seconds, state.seek_epoch, state.track_ended)
                                                };
                                                cached_seconds = cs;
                                                
                                                if track_ended || ce > local_epoch {
                                                    skip_push = true;
                                                    break;
                                                }
                                                
                                                if pts < cached_seconds - 0.05 {
                                                    skip_push = true;
                                                    break;
                                                }
                                                
                                                if pts <= cached_seconds + 0.05 {
                                                    break;
                                                }
                                                
                                                if sync_start.elapsed() > Duration::from_millis(500) {
                                                    println!("[Video Thread] Sync safety timeout reached for PTS={:.3}, cached_seconds={:.3}", pts, cached_seconds);
                                                    skip_push = true;
                                                    break;
                                                }
                                                std::thread::sleep(Duration::from_millis(2));
                                            }
                                        }
                                        
                                        println!("[Video Thread] Sync result for PTS {:.3}: cached_seconds={:.3}, skip_push={}", pts, cached_seconds, skip_push);
                                        if skip_push {
                                            continue;
                                        }
                                        
                                        match free_video_frame_rx.recv_timeout(Duration::from_millis(100)) {
                                            Ok(mut frame) => {
                                                frame.pts = pts;
                                                frame.width = decoded.width();
                                                frame.height = decoded.height();
                                                let send_res = video_frame_tx.try_send(frame);
                                                if let Err(e) = send_res {
                                                    println!("[Video Thread] try_send failed: {:?}", e);
                                                } else {
                                                    println!("[Video Thread] try_send succeeded for PTS: {:.3}", pts);
                                                }
                                            }
                                            Err(e) => {
                                                println!("[Video Thread] Failed to receive free frame from pool: {:?}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // Print error for debug if it's not EAGAIN (which typically has a specific debug representation)
                                        let err_str = format!("{:?}", e);
                                        if !err_str.contains("EAGAIN") && !err_str.contains("WouldBlock") {
                                            println!("[Video Thread] receive_frame error: {}", err_str);
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            println!("[Video Thread] Exiting");
        });
    }
    
    // Spawn Audio Decode Thread (mimicking main.rs/audio.rs decoder loop)
    let state_for_audio = shared_state.clone();
    let audio_thread = std::thread::spawn(move || {
        println!("[Audio Thread] Spawned");
        let hardware_channels = 2;
        let chunk_frames = 1024;
        let mut sample_buf = vec![0.0; chunk_frames * hardware_channels];
        
        let mut total_frames_read = 0;
        let start = Instant::now();
        
        loop {
            // Check for seek request
            let mut seek_request = false;
            {
                let mut state = state_for_audio.lock().unwrap();
                if let Some(pos) = state.seek_request.take() {
                    audio_source.set_position_seconds(pos);
                    state.current_seconds = pos;
                    state.seek_epoch += 1;
                    total_frames_read = (pos * 48000.0) as usize;
                    seek_request = true;
                }
            }
            
            let frames_read = audio_source.read_frames(hardware_channels, 48000, &mut sample_buf);
            if seek_request {
                println!("[Audio Thread] First read_frames after seek returned {} frames", frames_read);
            }
            if frames_read == 0 {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            
            total_frames_read += frames_read;
            let current_seconds = total_frames_read as f64 / 48000.0;
            {
                let mut state = state_for_audio.lock().unwrap();
                state.current_seconds = current_seconds;
            }
            
            if total_frames_read % (48000 * 5) == 0 {
                println!("[Audio Thread] Decoded {:.2}s", current_seconds);
            }
            
            std::thread::sleep(Duration::from_millis(20));
            
            if start.elapsed() > Duration::from_secs(8) {
                println!("[Audio Thread] Limit reached, exiting");
                break;
            }
        }
    });
    
    // UI Thread with seek insertion
    let state_for_ui = shared_state.clone();
    let ui_thread = std::thread::spawn(move || {
        println!("[UI Thread] Spawned");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(9) {
            // Simulate rapid scrubbing from 2.0s to 5.0s of elapsed time
            let elapsed = start.elapsed();
            if elapsed > Duration::from_secs(2) && elapsed < Duration::from_secs(2 + 1) {
                // Seek to bad/truncated part (80s)
                println!("[UI Thread] Seeking to 80.0s (corrupt/truncated)...");
                {
                    let mut state = state_for_ui.lock().unwrap();
                    state.seek_request = Some(80.0);
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
            if elapsed > Duration::from_secs(4) && elapsed < Duration::from_secs(4 + 1) {
                // Seek back to valid part (10s)
                println!("[UI Thread] Seeking back to 10.0s (valid)...");
                {
                    let mut state = state_for_ui.lock().unwrap();
                    state.seek_request = Some(10.0);
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
            
            let rx_opt = {
                let state = state_for_ui.lock().unwrap();
                state.video_frame_rx.clone()
            };
            
            if let Some(rx) = rx_opt {
                let mut latest_frame = None;
                let mut received_count = 0;
                while let Ok(frame) = rx.try_recv() {
                    received_count += 1;
                    if let Some(old_frame) = latest_frame.take() {
                        let tx = state_for_ui.lock().unwrap().free_video_frame_tx.clone().unwrap();
                        let _ = tx.try_send(old_frame);
                    }
                    latest_frame = Some(frame);
                }
                
                if received_count > 0 {
                    println!("[UI Thread] try_recv received {} frames", received_count);
                }
                
                if let Some(frame) = latest_frame {
                    println!("[UI Thread] Rendered video frame at PTS: {:.3}", frame.pts);
                    let tx = state_for_ui.lock().unwrap().free_video_frame_tx.clone().unwrap();
                    let send_res = tx.try_send(frame);
                    if send_res.is_err() {
                        println!("[UI Thread] Failed to return frame to pool: {:?}", send_res);
                    }
                }
            } else {
                println!("[UI Thread] rx_opt is None!");
            }
            
            std::thread::sleep(Duration::from_millis(33));
        }
        println!("[UI Thread] Exiting");
    });
    
    audio_thread.join().unwrap();
    ui_thread.join().unwrap();
    println!("Done!");
    Ok(())
}
