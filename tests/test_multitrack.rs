#[path = "../src/bitstream.rs"]
pub mod bitstream;
#[path = "../src/lyrics.rs"]
pub mod lyrics;
#[path = "../src/audio.rs"]
pub mod audio;
#[path = "../src/state.rs"]
pub mod state;

#[test]
fn test_multitrack_source_discovery_and_switching() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let source_res = audio::load_audio_source(test_file);
    assert!(source_res.is_ok(), "Failed to load test MKV: {:?}", source_res.err());
    let mut source = source_res.unwrap();

    let tracks = source.get_audio_tracks();
    println!("Discovered {} audio tracks:", tracks.len());
    for (idx, track) in tracks.iter().enumerate() {
        println!("  [{}] Stream #{}: {} (codec: {}, channels: {}, rate: {}Hz, lang: {:?})",
            idx, track.id, track.title, track.codec, track.channels, track.sample_rate, track.language
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
        assert!(switch_res.is_ok(), "Failed to switch to audio track {}: {:?}", idx, switch_res.err());
        assert_eq!(source.get_selected_audio_track(), idx);
        assert_eq!(source.get_type(), track.codec);

        let frames = source.read_frames(2, 48000, &mut buffer);
        println!("Track {} ({}): read {} frames", idx, track.codec, frames);
        assert!(frames > 0, "Failed to read audio frames after switching to track {}", idx);
    }
}

#[test]
fn test_multitrack_playback_switching_via_state() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new("Test App".to_string())));

    let handle_res = audio::start_audio_thread(test_file, false, shared_state.clone());
    assert!(handle_res.is_ok(), "Failed to start audio thread: {:?}", handle_res.err());
    let _handle = handle_res.unwrap();

    // Give the thread a moment to start and populate state
    std::thread::sleep(std::time::Duration::from_millis(200));

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(state.audio_tracks.len(), 4, "State should have 4 audio tracks");
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
    assert!(switched, "Decoder thread did not switch to audio track 2 in time");

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
    assert!(switched_back, "Decoder thread did not switch back to audio track 0 in time");
}

#[test]
fn test_singletrack_no_op_and_position_preservation() {
    let test_file = "audio_tests/sine_sweep_ac3_5.1.ac3";
    if std::path::Path::new(test_file).exists() {
        let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new("Test App".to_string())));

        let handle_res = audio::start_audio_thread(test_file, false, shared_state.clone());
        assert!(handle_res.is_ok(), "Failed to start audio thread: {:?}", handle_res.err());
        let _handle = handle_res.unwrap();

        std::thread::sleep(std::time::Duration::from_millis(200));

        {
            let state = shared_state.lock().unwrap();
            assert_eq!(state.audio_tracks.len(), 1, "Single-stream file should have exactly 1 track");
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
            assert_eq!(state.seek_epoch, initial_epoch, "seek_epoch should not increment on redundant/single-track switch");
        }
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
    assert!(source_res.is_ok(), "Failed to load test MKV: {:?}", source_res.err());
    let mut source = source_res.unwrap();

    let tracks = source.get_audio_tracks();
    assert!(tracks.len() >= 2, "Test requires at least 2 tracks");

    // Mix Track 0 and Track 1 with equal volume
    let mix_res = source.set_active_audio_tracks(&[(0, 1.0), (1, 1.0)]);
    assert!(mix_res.is_ok(), "Failed to set active audio tracks: {:?}", mix_res.err());

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
    assert!(frames_after_adjust > 0, "Failed to read frames after volume adjustment");
}

#[test]
fn test_multitrack_waveform_timeline_on_track_switch() {
    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new("Test App".to_string())));
    {
        let mut state = shared_state.lock().unwrap();
        state.is_paused = false;
    }

    let handle_res = audio::start_audio_thread(test_file, false, shared_state.clone());
    assert!(handle_res.is_ok(), "Failed to start audio thread: {:?}", handle_res.err());
    let _handle = handle_res.unwrap();

    // Allow initial playback to decode and push lookahead slices
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify initial timeline has waveform data and queue is monotonic
    {
        let state = shared_state.lock().unwrap();
        assert!(!state.lookahead_queue.is_empty(), "Lookahead queue should be populated initially");
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(*t > prev_t, "Lookahead queue timestamps must be strictly monotonic: {} <= {}", *t, prev_t);
            prev_t = *t;
        }
        let max_amp = state.lookahead_timeline.iter().fold(0.0f32, |acc, &v| acc.max(v.abs()));
        assert!(max_amp > 0.0, "Initial lookahead timeline must contain non-zero audio waveform data");
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
        assert!(!state.lookahead_queue.is_empty(), "Lookahead queue should be populated on track 2");
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(*t > prev_t, "Lookahead queue timestamps must be strictly monotonic on track 2: {} <= {}", *t, prev_t);
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
        assert!(!state.lookahead_queue.is_empty(), "Lookahead queue must be populated after switching back to track 0");
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(*t > prev_t, "Lookahead queue timestamps must be strictly monotonic after switching back: {} <= {}", *t, prev_t);
            prev_t = *t;
        }
        let max_amp = state.lookahead_timeline.iter().fold(0.0f32, |acc, &v| acc.max(v.abs()));
        println!("Track 0 after switch-back max timeline amplitude: {}", max_amp);
        assert!(max_amp > 0.0, "Timeline must contain non-zero audio waveform data (no flatline!)");
    }
}

#[test]
fn test_canto_multitrack_isolation_and_mixing() {
    let test_file = "/home/naoki/.gemini/antigravity/brain/ed1be029-3831-4402-9794-356b0a866606/scratch/canto_test.mp4";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let source_res = audio::load_audio_source(test_file);
    assert!(source_res.is_ok(), "Failed to load canto test: {:?}", source_res.err());
    let mut source = source_res.unwrap();
    let tracks = source.get_audio_tracks();
    println!("Discovered {} tracks in canto_test.mp4:", tracks.len());
    for (i, t) in tracks.iter().enumerate() {
        println!("  [{}] Title: '{}', Codec: {}, Channels: {}", i, t.title, t.codec, t.channels);
    }

    assert_eq!(tracks.len(), 2, "Expected 2 tracks");

    // 1. Select Track 0 (Instrumental) only, at intro (pos 2.0s)
    source.select_audio_track(0).unwrap();
    source.set_position_seconds(2.0);
    let mut buf0 = vec![0.0f32; 44100 * 2]; // 1 second of stereo
    let n0 = source.read_frames(2, 44100, &mut buf0);
    assert!(n0 > 0);
    let rms0 = (buf0[..n0 * 2].iter().map(|&s| s * s).sum::<f32>() / (n0 * 2) as f32).sqrt();
    println!("Track 0 intro RMS: {:.4}", rms0);
    assert!(rms0 > 0.05, "Track 0 (Instrumental) should have active music during intro");

    // 2. Select Track 1 (Vocals) only, at intro (pos 2.0s)
    source.select_audio_track(1).unwrap();
    source.set_position_seconds(2.0);
    let mut buf1_intro = vec![0.0f32; 44100 * 2];
    let n1_intro = source.read_frames(2, 44100, &mut buf1_intro);
    assert!(n1_intro > 0);
    let rms1_intro = (buf1_intro[..n1_intro * 2].iter().map(|&s| s * s).sum::<f32>() / (n1_intro * 2) as f32).sqrt();
    println!("Track 1 intro RMS (vocals guide stem): {:.6}", rms1_intro);
    assert!(rms1_intro < 0.005, "Track 1 (Vocals) should be virtually SILENT during the intro! Got: {}", rms1_intro);

    // 3. Select Track 1 (Vocals) only, at verse (pos 52.0s) where singing starts
    source.select_audio_track(1).unwrap();
    source.set_position_seconds(52.0);
    let mut buf1_verse = vec![0.0f32; 44100 * 2];
    let n1_verse = source.read_frames(2, 44100, &mut buf1_verse);
    assert!(n1_verse > 0);
    let rms1_verse = (buf1_verse[..n1_verse * 2].iter().map(|&s| s * s).sum::<f32>() / (n1_verse * 2) as f32).sqrt();
    println!("Track 1 verse RMS (singing active at 52s): {:.4}", rms1_verse);
    assert!(rms1_verse > 0.05, "Track 1 (Vocals) should have active singing during the verse");

    // 4. Mix both tracks 0 + 1 (50% music, 100% vocals)
    source.set_active_audio_tracks(&[(0, 0.5), (1, 1.0)]).unwrap();
    source.set_position_seconds(52.0);
    let mut buf_mix = vec![0.0f32; 44100 * 2];
    let n_mix = source.read_frames(2, 44100, &mut buf_mix);
    assert!(n_mix > 0);
    let rms_mix = (buf_mix[..n_mix * 2].iter().map(|&s| s * s).sum::<f32>() / (n_mix * 2) as f32).sqrt();
    println!("Track 0+1 mix RMS: {:.4}", rms_mix);
    assert!(rms_mix > 0.05, "Mixed audio should be non-zero and active");
}

#[test]
fn test_canto_playback_state_switching() {
    let test_file = "/home/naoki/.gemini/antigravity/brain/ed1be029-3831-4402-9794-356b0a866606/scratch/canto_test.mp4";
    if !std::path::Path::new(test_file).exists() {
        eprintln!("Skipping test: test file {} does not exist", test_file);
        return;
    }
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new("Test App".to_string())));
    let handle_res = audio::start_audio_thread(test_file, false, shared_state.clone());
    assert!(handle_res.is_ok(), "Failed to start audio thread: {:?}", handle_res.err());
    let _handle = handle_res.unwrap();

    std::thread::sleep(std::time::Duration::from_millis(250));

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(state.audio_tracks.len(), 2);
        assert_eq!(state.selected_audio_track, 0);
        assert!(state.audio_tracks[0].title.to_lowercase().contains("instrumental") || state.audio_tracks[0].title.to_lowercase().contains("karaoke"));
        assert!(state.audio_tracks[1].title.to_lowercase().contains("vocal") || state.audio_tracks[1].title.to_lowercase().contains("guide"));
    }

    // Simulate user clicking Track 2 (Vocals) in the GUI:
    // main.rs sets selected_audio_track, active_audio_tracks, audio_track_request, and audio_mix_request
    {
        let mut state = shared_state.lock().unwrap();
        state.selected_audio_track = 1;
        state.active_audio_tracks = vec![1];
        state.audio_track_request = Some(1);
        state.audio_mix_request = Some(vec![(1, 1.0)]);
    }

    // Wait for the decoder thread to process
    let mut switched = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == 1 && state.active_audio_tracks == vec![1] {
            switched = true;
            break;
        }
    }
    assert!(switched, "Decoder did not switch to track 1");

    // Simulate user clicking "Mix 1&2" in GUI
    {
        let mut state = shared_state.lock().unwrap();
        state.active_audio_tracks = vec![0, 1];
        state.audio_mix_request = Some(vec![(0, 1.0), (1, 1.0)]);
    }

    let mut mixed = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.active_audio_tracks == vec![0, 1] && state.multi_track_mix_mode {
            mixed = true;
            break;
        }
    }
    assert!(mixed, "Decoder did not switch to mix 0+1");
}

#[test]
fn test_symphonia_canto_mp4() {
    let test_file = "/home/naoki/.gemini/antigravity/brain/ed1be029-3831-4402-9794-356b0a866606/scratch/canto_test.mp4";
    if !std::path::Path::new(test_file).exists() {
        return;
    }
    let file = std::fs::File::open(test_file).unwrap();
    let res = audio::try_symphonia(file, "mp4", "mp4", None, Some(test_file));
    println!("Symphonia load result: {:?}", res.is_ok());
    if let Err(ref e) = res {
        println!("Symphonia load error: {:?}", e);
    }
    assert!(res.is_ok(), "Symphonia must be able to load the Canto MP4 file!");
    let mut source = res.unwrap();
    let tracks = source.get_audio_tracks();
    println!("Symphonia discovered {} tracks:", tracks.len());
    for (i, t) in tracks.iter().enumerate() {
        println!("  [{}] Title: '{}', Codec: {}, Channels: {}", i, t.title, t.codec, t.channels);
    }

    // 1. Select Track 0 only, seek to 52s, read frames
    source.select_audio_track(0).unwrap();
    source.set_position_seconds(52.0);
    let mut buf0 = vec![0.0f32; 44100 * 2];
    let n0 = source.read_frames(2, 44100, &mut buf0);
    println!("Symphonia Track 0 at 52s read {} frames", n0);
    let rms0 = (buf0[..n0 * 2].iter().map(|&s| s * s).sum::<f32>() / (n0 * 2) as f32).sqrt();
    println!("Symphonia Track 0 RMS: {:.4}", rms0);

    // 2. Select Track 1 only, seek to 52s, read frames
    source.select_audio_track(1).unwrap();
    source.set_position_seconds(52.0);
    let mut buf1 = vec![0.0f32; 44100 * 2];
    let n1 = source.read_frames(2, 44100, &mut buf1);
    println!("Symphonia Track 1 at 52s read {} frames", n1);
    let rms1 = (buf1[..n1 * 2].iter().map(|&s| s * s).sum::<f32>() / (n1 * 2) as f32).sqrt();
    println!("Symphonia Track 1 RMS: {:.4}", rms1);

    // 3. Select Track 1 only, seek to 2s (intro), read frames
    source.select_audio_track(1).unwrap();
    source.set_position_seconds(2.0);
    let mut buf1_intro = vec![0.0f32; 44100 * 2];
    let n1_intro = source.read_frames(2, 44100, &mut buf1_intro);
    let rms1_intro = (buf1_intro[..n1_intro * 2].iter().map(|&s| s * s).sum::<f32>() / (n1_intro * 2) as f32).sqrt();
    println!("Symphonia Track 1 intro RMS: {:.6}", rms1_intro);
}

#[test]
fn test_wicked_game_full_mix_cancellation() {
    let test_file = "/home/naoki/.gemini/antigravity/brain/ed1be029-3831-4402-9794-356b0a866606/scratch/wicked_game.mp4";
    if !std::path::Path::new(test_file).exists() {
        return;
    }

    // Test with Symphonia (used on Android)
    let file = std::fs::File::open(test_file).unwrap();
    let sym_res = audio::try_symphonia(file, "mp4", "mp4", None, Some(test_file));
    assert!(sym_res.is_ok(), "Symphonia must load wicked_game.mp4");
    let mut sym_source = sym_res.unwrap();
    let sym_tracks = sym_source.get_audio_tracks();
    assert_eq!(sym_tracks.len(), 2);
    println!("Wicked Game Symphonia tracks:");
    for (i, t) in sym_tracks.iter().enumerate() {
        println!("  [{}] Title: '{}'", i, t.title);
    }

    // 1. In intro (18s), Track 0 (Karaoke) should be loud
    sym_source.select_audio_track(0).unwrap();
    sym_source.set_position_seconds(18.0);
    let mut buf0 = vec![0.0f32; 44100 * 2];
    let n0 = sym_source.read_frames(2, 44100, &mut buf0);
    let rms0 = (buf0[..n0 * 2].iter().map(|&s| s * s).sum::<f32>() / (n0 * 2) as f32).sqrt();
    println!("Symphonia Track 0 intro (18s) RMS: {:.4}", rms0);
    assert!(rms0 > 0.05, "Track 0 should have active guitar/drums during intro");

    // 2. In intro (18s), Track 1 (Solo Vocals) should be virtually SILENT due to phase cancellation!
    sym_source.select_audio_track(1).unwrap();
    sym_source.set_position_seconds(18.0);
    let mut buf1_intro = vec![0.0f32; 44100 * 2];
    let n1 = sym_source.read_frames(2, 44100, &mut buf1_intro);
    let rms1_intro = (buf1_intro[..n1 * 2].iter().map(|&s| s * s).sum::<f32>() / (n1 * 2) as f32).sqrt();
    println!("Symphonia Track 1 intro (18s) RMS: {:.6}", rms1_intro);
    assert!(rms1_intro < 0.005, "Track 1 intro must be silent due to phase cancellation! Got {}", rms1_intro);

    // 3. In verse (40s), Track 1 (Solo Vocals) should have isolated vocals!
    sym_source.select_audio_track(1).unwrap();
    sym_source.set_position_seconds(40.0);
    let mut buf1_verse = vec![0.0f32; 44100 * 2];
    let n1_v = sym_source.read_frames(2, 44100, &mut buf1_verse);
    let rms1_verse = (buf1_verse[..n1_v * 2].iter().map(|&s| s * s).sum::<f32>() / (n1_v * 2) as f32).sqrt();
    println!("Symphonia Track 1 verse (40s) RMS: {:.4}", rms1_verse);
    assert!(rms1_verse > 0.02, "Track 1 verse must have vocals!");

    // Also test with FFmpeg source (desktop)
    let ff_res = audio::load_audio_source(test_file);
    assert!(ff_res.is_ok(), "FFmpeg must load wicked_game.mp4");
    let mut ff_source = ff_res.unwrap();
    ff_source.select_audio_track(1).unwrap();
    ff_source.set_position_seconds(18.0);
    let mut ff_buf1 = vec![0.0f32; 48000 * 2];
    let ff_n = ff_source.read_frames(2, 48000, &mut ff_buf1);
    let ff_rms1 = (ff_buf1[..ff_n * 2].iter().map(|&s| s * s).sum::<f32>() / (ff_n * 2) as f32).sqrt();
    println!("FFmpeg Track 1 intro (18s) RMS: {:.6}", ff_rms1);
    assert!(ff_rms1 < 0.005, "FFmpeg Track 1 intro must be silent due to phase cancellation! Got {}", ff_rms1);
}

#[test]
fn test_continuous_playback_wicked_game() {
    let test_file = "/home/naoki/.gemini/antigravity/brain/ed1be029-3831-4402-9794-356b0a866606/scratch/wicked_game.mp4";
    if !std::path::Path::new(test_file).exists() { return; }
    let file = std::fs::File::open(test_file).unwrap();
    let mut sym = audio::try_symphonia(file, "mp4", "mp4", None, Some(test_file)).unwrap();
    sym.select_audio_track(1).unwrap();
    sym.set_position_seconds(30.0);
    let mut buf = vec![0.0f32; 1024 * 2];
    for sec in 0..15 {
        let mut total_rms = 0.0f32;
        let mut total_frames = 0;
        for _ in 0..43 {
            let n = sym.read_frames(2, 44100, &mut buf);
            if n > 0 {
                let rms = (buf[..n * 2].iter().map(|&s| s * s).sum::<f32>() / (n * 2) as f32).sqrt();
                total_rms += rms;
                total_frames += n;
            }
        }
        println!("At time ~{}s ({} frames read): avg RMS = {:.5}", 30 + sec, total_frames, total_rms / 43.0);
    }
}
