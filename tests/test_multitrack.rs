use rusttracker::{audio, state};

#[test]
fn test_multitrack_source_discovery_and_switching() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let source_res = audio::load_audio_source(test_file);
    assert!(
        source_res.is_ok(),
        "Failed to load test MKV: {:?}",
        source_res.err()
    );
    let mut source = source_res.unwrap();

    let tracks = source.get_audio_tracks();
    println!("Discovered {} audio tracks:", tracks.len());
    for (idx, track) in tracks.iter().enumerate() {
        println!(
            "  [{}] Stream #{}: {} (codec: {}, channels: {}, rate: {}Hz, lang: {:?})",
            idx,
            track.id,
            track.title,
            track.codec,
            track.channels,
            track.sample_rate,
            track.language
        );
    }

    assert_eq!(tracks.len(), 4, "Expected 4 audio tracks in test MKV");
    assert_eq!(tracks[0].codec, "TrueHD");
    assert_eq!(tracks[1].codec, "AC3");
    assert_eq!(tracks[2].codec, "E-AC3");
    assert_eq!(tracks[3].codec, "AC3");

    // Test switching to each track and reading audio frames
    let mut buffer = vec![0.0f32; 1024 * 2];
    for (idx, track) in tracks.iter().enumerate() {
        let switch_res = source.select_audio_track(idx);
        assert!(
            switch_res.is_ok(),
            "Failed to switch to audio track {}: {:?}",
            idx,
            switch_res.err()
        );
        assert_eq!(source.get_selected_audio_track(), idx);
        assert_eq!(source.get_type(), track.codec);

        let frames = source.read_frames(2, 48000, &mut buffer);
        println!("Track {} ({}): read {} frames", idx, track.codec, frames);
        assert!(
            frames > 0,
            "Failed to read audio frames after switching to track {}",
            idx
        );
    }
}

#[test]
fn test_multitrack_playback_switching_via_state() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));

    let handle_res = audio::start_audio_thread(test_file, false, shared_state.clone());
    assert!(
        handle_res.is_ok(),
        "Failed to start audio thread: {:?}",
        handle_res.err()
    );
    let _handle = handle_res.unwrap();

    // Give the thread a moment to start and populate state
    std::thread::sleep(std::time::Duration::from_millis(200));

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(
            state.audio_tracks.len(),
            4,
            "State should have 4 audio tracks"
        );
        println!("Initial selected track: {}", state.selected_audio_track);
    }

    // Request switch to track 2 (E-AC3)
    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(2);
    }

    // Wait for the decoder thread to process the request
    let mut switched = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == 2 {
            switched = true;
            assert_eq!(state.module_type, "E-AC3");
            break;
        }
    }
    assert!(
        switched,
        "Decoder thread did not switch to audio track 2 in time"
    );

    // Request switch back to track 0 (TrueHD)
    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(0);
    }

    let mut switched_back = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == 0 {
            switched_back = true;
            assert_eq!(state.module_type, "TrueHD");
            break;
        }
    }
    assert!(
        switched_back,
        "Decoder thread did not switch back to audio track 0 in time"
    );
}

#[test]
fn test_singletrack_no_op_and_position_preservation() {
    let test_file = "audio_tests/FLAC 5.1.flac";
    assert!(
        std::path::Path::new(test_file).exists(),
        "Expected fixture {} to exist",
        test_file
    );
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));

    let _handle = audio::start_audio_thread(test_file, false, shared_state.clone())
        .expect("Failed to start audio thread");

    std::thread::sleep(std::time::Duration::from_millis(200));

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(
            state.audio_tracks.len(),
            1,
            "Single-stream file should have exactly 1 track"
        );
        assert_eq!(state.selected_audio_track, 0);
    }

    // Setting audio_track_request on single track should be ignored and not advance seek_epoch
    let initial_epoch = {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(0);
        state.seek_epoch
    };

    std::thread::sleep(std::time::Duration::from_millis(100));

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(
            state.seek_epoch, initial_epoch,
            "seek_epoch should not increment on redundant/single-track switch"
        );
    }
}

#[test]
fn test_multitrack_mixing_and_volume_blending() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let source_res = audio::load_audio_source(test_file);
    assert!(
        source_res.is_ok(),
        "Failed to load test MKV: {:?}",
        source_res.err()
    );
    let mut source = source_res.unwrap();

    let tracks = source.get_audio_tracks();
    assert!(tracks.len() >= 2, "Test requires at least 2 tracks");

    // Mix Track 0 and Track 1 with equal volume
    let mix_res = source.set_active_audio_tracks(&[(0, 1.0), (1, 1.0)]);
    assert!(
        mix_res.is_ok(),
        "Failed to set active audio tracks: {:?}",
        mix_res.err()
    );

    let active_tracks = source.get_active_audio_tracks();
    assert_eq!(active_tracks.len(), 2, "Should have 2 active tracks");
    assert!(active_tracks.contains(&0));
    assert!(active_tracks.contains(&1));

    let mut buffer = vec![0.0f32; 1024 * 2];
    let frames = source.read_frames(2, 48000, &mut buffer);
    println!("Read {} mixed frames", frames);
    assert!(frames > 0, "Failed to read frames from mixed streams");

    // Adjust volume (mute track 1, keep track 0)
    let adjust_res = source.set_active_audio_tracks(&[(0, 1.0), (1, 0.0)]);
    assert!(adjust_res.is_ok(), "Failed to adjust track volumes");

    let frames_after_adjust = source.read_frames(2, 48000, &mut buffer);
    assert!(
        frames_after_adjust > 0,
        "Failed to read frames after volume adjustment"
    );
}

#[test]
fn test_multitrack_waveform_timeline_on_track_switch() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));
    {
        let mut state = shared_state.lock().unwrap();
        state.is_paused = false;
    }

    let handle_res = audio::start_audio_thread(test_file, false, shared_state.clone());
    assert!(
        handle_res.is_ok(),
        "Failed to start audio thread: {:?}",
        handle_res.err()
    );
    let _handle = handle_res.unwrap();

    // Allow initial playback to decode and push lookahead slices
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify initial timeline has waveform data and queue is monotonic
    {
        let state = shared_state.lock().unwrap();
        assert!(
            !state.lookahead_queue.is_empty(),
            "Lookahead queue should be populated initially"
        );
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(
                *t > prev_t,
                "Lookahead queue timestamps must be strictly monotonic: {} <= {}",
                *t,
                prev_t
            );
            prev_t = *t;
        }
        let max_amp = state
            .lookahead_timeline
            .iter()
            .fold(0.0f32, |acc, &v| acc.max(v.abs()));
        assert!(
            max_amp > 0.0,
            "Initial lookahead timeline must contain non-zero audio waveform data"
        );
    }

    // Switch to Track 2 (E-AC3)
    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(2);
    }

    let mut switched_to_2 = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == 2 {
            switched_to_2 = true;
            break;
        }
    }
    assert!(switched_to_2, "Failed to switch to track 2");

    // Allow decoder and DSP thread to process Track 2
    std::thread::sleep(std::time::Duration::from_millis(400));

    {
        let state = shared_state.lock().unwrap();
        assert!(
            !state.lookahead_queue.is_empty(),
            "Lookahead queue should be populated on track 2"
        );
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(
                *t > prev_t,
                "Lookahead queue timestamps must be strictly monotonic on track 2: {} <= {}",
                *t,
                prev_t
            );
            prev_t = *t;
        }
    }

    // Switch back to Track 0 (Track 1)
    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(0);
    }

    let mut switched_back_to_0 = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == 0 {
            switched_back_to_0 = true;
            break;
        }
    }
    assert!(switched_back_to_0, "Failed to switch back to track 0");

    // Allow decoder and DSP thread to populate lookahead slices after switching back
    std::thread::sleep(std::time::Duration::from_millis(500));

    {
        let state = shared_state.lock().unwrap();
        assert!(
            !state.lookahead_queue.is_empty(),
            "Lookahead queue must be populated after switching back to track 0"
        );
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(
                *t > prev_t,
                "Lookahead queue timestamps must be strictly monotonic after switching back: {} <= {}",
                *t,
                prev_t
            );
            prev_t = *t;
        }
        let max_amp = state
            .lookahead_timeline
            .iter()
            .fold(0.0f32, |acc, &v| acc.max(v.abs()));
        println!(
            "Track 0 after switch-back max timeline amplitude: {}",
            max_amp
        );
        assert!(
            max_amp > 0.0,
            "Timeline must contain non-zero audio waveform data (no flatline!)"
        );
    }
}

#[test]
fn test_itu_bs775_downmix_all_channel_layouts() {
    // 1. 3.0 / 2.1 (FL, FR, FC/LFE) -> Stereo & Mono
    let ch3 = [0.6f32, 0.2, 0.4];
    let l3 = audio::get_downmixed_sample(3, &ch3, 2, 0);
    let r3 = audio::get_downmixed_sample(3, &ch3, 2, 1);
    let m3 = audio::get_downmixed_sample(3, &ch3, 1, 0);
    assert!(l3 > r3, "Expected FL+FC > FR+FC for 3.0 downmix");
    assert!(m3 > 0.0, "Expected non-zero mono downmix for 3.0");

    // 2. 4.0 Quad (FL, FR, BL, BR) -> Stereo & Mono
    let ch4 = [0.5f32, 0.1, 0.4, 0.1];
    let l4 = audio::get_downmixed_sample(4, &ch4, 2, 0);
    let r4 = audio::get_downmixed_sample(4, &ch4, 2, 1);
    let m4 = audio::get_downmixed_sample(4, &ch4, 1, 0);
    assert!(l4 > r4, "Expected left-heavy quad downmix to have L > R");
    assert!(
        (m4 - 0.275).abs() < 1e-4,
        "Expected exact quad mono average"
    );

    // 3. 5.0 (FL, FR, FC, BL, BR) -> Stereo & Mono
    let ch5 = [0.5f32, 0.2, 0.3, 0.4, 0.1];
    let l5 = audio::get_downmixed_sample(5, &ch5, 2, 0);
    let r5 = audio::get_downmixed_sample(5, &ch5, 2, 1);
    let m5 = audio::get_downmixed_sample(5, &ch5, 1, 0);
    assert!(l5 > r5, "Expected left-heavy 5.0 downmix to have L > R");
    assert!(m5 > 0.0, "Expected non-zero 5.0 mono downmix");

    // 4. 5.1 (FL, FR, FC, LFE, BL, BR) -> Stereo & Mono
    let ch6 = [0.5f32, 0.2, 0.3, 0.2, 0.4, 0.1];
    let l6 = audio::get_downmixed_sample(6, &ch6, 2, 0);
    let r6 = audio::get_downmixed_sample(6, &ch6, 2, 1);
    let m6 = audio::get_downmixed_sample(6, &ch6, 1, 0);
    assert!(l6 > r6, "Expected left-heavy 5.1 downmix to have L > R");
    assert!(m6 > 0.0, "Expected non-zero 5.1 mono downmix");

    // 5. 6.1 (FL, FR, FC, LFE, BC, SL, SR) -> Stereo & Mono
    let ch7 = [0.5f32, 0.2, 0.3, 0.1, 0.2, 0.4, 0.1];
    let l7 = audio::get_downmixed_sample(7, &ch7, 2, 0);
    let r7 = audio::get_downmixed_sample(7, &ch7, 2, 1);
    let m7 = audio::get_downmixed_sample(7, &ch7, 1, 0);
    assert!(l7 > r7, "Expected left-heavy 6.1 downmix to have L > R");
    assert!(m7 > 0.0, "Expected non-zero 6.1 mono downmix");

    // 6. 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR) -> Stereo & Mono
    let ch8 = [0.5f32, 0.2, 0.3, 0.1, 0.4, 0.1, 0.3, 0.1];
    let l8 = audio::get_downmixed_sample(8, &ch8, 2, 0);
    let r8 = audio::get_downmixed_sample(8, &ch8, 2, 1);
    let m8 = audio::get_downmixed_sample(8, &ch8, 1, 0);
    assert!(l8 > r8, "Expected left-heavy 7.1 downmix to have L > R");
    assert!(m8 > 0.0, "Expected non-zero 7.1 mono downmix");
}

#[test]
fn test_symphonia_7_1_flac_decoding_and_seeking() {
    let test_file = "audio_tests/FLAC 7.1.flac";
    assert!(
        std::path::Path::new(test_file).exists(),
        "Expected fixture {} to exist",
        test_file
    );
    let file = std::fs::File::open(test_file).expect("Failed to open FLAC 7.1.flac");
    let mut sym = audio::try_symphonia(file, "flac", "flac", None, Some(test_file))
        .expect("Symphonia must decode 7.1 FLAC");

    assert_eq!(sym.get_num_channels(), 8, "Expected 8 channels in 7.1 FLAC");
    assert!(sym.get_duration_seconds() > 1.0);

    let mut stereo_buf = vec![0.0f32; 1024 * 2];
    let n = sym.read_frames(2, 48_000, &mut stereo_buf);
    assert!(n > 0, "Expected decoded stereo frames from 7.1 FLAC");

    // Seek to 1.0s and verify continued decoding and per-channel VU metering
    sym.set_position_seconds(1.0);
    let n_after_seek = sym.read_frames(2, 48_000, &mut stereo_buf);
    assert!(
        n_after_seek > 0,
        "Expected decoded frames after seeking in 7.1 FLAC"
    );
    let mut max_ch_vu = 0.0f32;
    for ch in 0..8 {
        max_ch_vu = max_ch_vu.max(sym.get_current_channel_vu_mono(ch));
    }
    assert!(
        max_ch_vu > 0.0,
        "Expected non-zero per-channel VU on 7.1 FLAC"
    );
}
