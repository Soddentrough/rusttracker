use ffmpeg_next as ffmpeg;

#[test]
fn test_ffmpeg() {
    let _ = ffmpeg::init();
    let file_path = "audio_tests/AAC 5.1.mp4";
    let ictx = match ffmpeg::format::input(&file_path) {
        Ok(ictx) => ictx,
        Err(e) => {
            println!("Failed to open {}: {}", file_path, e);
            return;
        }
    };
    
    let video_stream = ictx.streams().best(ffmpeg::media::Type::Video);
    println!("Video stream: {}", video_stream.is_some());
    let audio_stream = ictx.streams().best(ffmpeg::media::Type::Audio);
    println!("Audio stream: {}", audio_stream.is_some());
}

#[test]
fn test_multichannel_dsp_pipeline() {
    let _ = ffmpeg::init();
    let file_path = "audio_tests/AAC 5.1.mp4";
    if !std::path::Path::new(file_path).exists() {
        println!("Track not found at {}", file_path);
        return;
    }
    let mut ictx = ffmpeg::format::input(&file_path).expect("Failed to open file");
    let (best_audio_index, parameters) = {
        let best_audio = ictx.streams().best(ffmpeg::media::Type::Audio).expect("No audio");
        (best_audio.index(), best_audio.parameters())
    };
    let decoder_context = ffmpeg::codec::context::Context::from_parameters(parameters).expect("Context");
    let mut decoder = decoder_context.decoder().audio().expect("Decoder");

    let mut vis_resampler = ffmpeg::software::resampling::context::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg::format::sample::Sample::F32(ffmpeg::format::sample::Type::Planar),
        decoder.channel_layout(),
        decoder.rate(),
    ).expect("Vis resampler get failed");

    let decoder_rate = decoder.rate() as f32;
    let window_size = rusttracker::audio::calculate_power_of_two_window_size(decoder.rate());
    let update_interval = (decoder_rate / 60.0).ceil() as usize;
    assert!(window_size.is_power_of_two(), "window_size {} must be power of two", window_size);

    let mut accumulator: Vec<Vec<f32>> = Vec::new();
    let mut samples_since_last_send = 0;
    let mut sent_count = 0;
    let mut state = rusttracker::state::AppState::new("test".to_string());
    state.stats.bitstream_active = true;

    let mut max_observed_peak = 0.0f32;
    let mut max_observed_mono = 0.0f32;

    for (stream, packet) in ictx.packets() {
        if stream.index() == best_audio_index {
            let current_seconds = packet.pts().map(|p| p as f64 * f64::from(stream.time_base())).unwrap_or(0.0);
            if decoder.send_packet(&packet).is_ok() {
                let mut frame = ffmpeg::frame::Audio::empty();
                while decoder.receive_frame(&mut frame).is_ok() {
                    let mut vis_frame = ffmpeg::frame::Audio::empty();
                    if vis_resampler.run(&frame, &mut vis_frame).is_ok() {
                        let planes = vis_frame.channels() as usize;
                        if accumulator.len() != planes {
                            accumulator = vec![Vec::new(); planes];
                        }
                        let mut fresh_samples = 0;
                        for (p, acc) in accumulator.iter_mut().enumerate().take(planes) {
                            let data = vis_frame.plane::<f32>(p);
                            if p == 0 { fresh_samples = data.len(); }
                            acc.extend_from_slice(data);
                            let excess = acc.len().saturating_sub(window_size);
                            if excess > 0 {
                                acc.drain(0..excess);
                            }
                        }
                        samples_since_last_send += fresh_samples;
                        if accumulator.first().map(|a| a.len()).unwrap_or(0) == window_size && samples_since_last_send >= update_interval {
                            samples_since_last_send = 0;
                            sent_count += 1;

                            let mut channel_audio_data = Vec::with_capacity(planes);
                            let mut channel_vus = Vec::with_capacity(planes);
                            let eval_len = fresh_samples.max(update_interval).min(window_size);

                            for acc in accumulator.iter().take(planes) {
                                let window = acc.clone();
                                let mut peak = 0.0f32;
                                let start_idx = window.len().saturating_sub(eval_len);
                                for &s in &window[start_idx..] {
                                    peak = peak.max(s.abs());
                                }
                                channel_vus.push(peak.clamp(0.0, 1.0));
                                channel_audio_data.push(window);
                            }

                            for &vu in &channel_vus {
                                max_observed_peak = max_observed_peak.max(vu);
                            }

                            let mut mono_audio_data = vec![0.0f32; window_size];
                            for acc in accumulator.iter().take(planes) {
                                for (i, &sample) in acc.iter().take(window_size).enumerate() {
                                    mono_audio_data[i] += sample;
                                }
                            }
                            let inv_planes = 1.0 / planes as f32;
                            for s in &mut mono_audio_data {
                                *s *= inv_planes;
                                max_observed_mono = max_observed_mono.max(s.abs());
                            }

                            // Test lookahead slice extraction
                            rusttracker::audio::push_planar_lookahead_slices(&mut state, &channel_audio_data, decoder.rate(), current_seconds);
                        }
                    }
                }
            }
        }
        if sent_count >= 60 {
            break;
        }
    }

    assert!(sent_count > 0, "Expected at least 1 DSP update sent, got {}", sent_count);
    assert!(max_observed_peak > 0.05, "Expected peak VU > 0.05, got {}", max_observed_peak);
    assert!(max_observed_mono > 0.01, "Expected mono audio signal > 0.01, got {}", max_observed_mono);
    assert!(!state.lookahead_queue.is_empty(), "Expected lookahead_queue to be populated");
    let has_lookahead_audio = state.lookahead_queue.iter().any(|(_, d)| d[0].abs() > 0.0 || d[1].abs() > 0.0 || d[4] > 0.0);
    assert!(has_lookahead_audio, "Expected non-zero audio metrics in lookahead_queue");
}

#[test]
fn test_aac_5_1_downmix() {
    let file_path = "audio_tests/AAC 5.1.mp4";
    if !std::path::Path::new(file_path).exists() {
        return;
    }
    let mut source = rusttracker::audio::load_audio_source(file_path).expect("Failed to load AAC 5.1.mp4");
    assert_eq!(source.get_num_channels(), 6);

    let mut stereo_buf = vec![0.0f32; 1024 * 2];
    let frames = source.read_frames(2, 48000, &mut stereo_buf);
    assert!(frames > 0, "Expected frames read > 0");
    assert!(stereo_buf.iter().any(|&s| s.abs() > 0.0), "Expected non-silent downmixed stereo audio");
}

#[test]
fn test_video_rotation_90() {
    for file_path in ["audio_tests/portrait_test_90.mp4", "audio_tests/portrait_test_90.mkv"] {
        if !std::path::Path::new(file_path).exists() {
            continue;
        }
        let mut source = rusttracker::audio::load_audio_source(file_path)
            .unwrap_or_else(|e| panic!("Failed to load {}: {}", file_path, e));
        
        // Video info should show rotated dimensions (360x640 instead of 640x360)
        let info = source.get_video_info().expect("Expected video info");
        assert!(info.contains("360x640"), "Expected rotated dimensions 360x640 for {}, got: {}", file_path, info);

        // take_video_parameters should report rotation 90
        let (_params, _tb, rotation) = source.take_video_parameters().expect("Expected video parameters");
        assert_eq!(rotation, 90, "Expected rotation == 90 degrees for {}", file_path);
    }
}


