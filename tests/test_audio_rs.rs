use rusttracker::{audio, lyrics, state};

#[test]
fn test_midi_loading_and_tempo_map() {
    let test_file = "audio_tests/darude-sandstorm.mid";
    assert!(
        std::path::Path::new(test_file).exists(),
        "Expected MIDI fixture {} to exist",
        test_file
    );
    let mut source = audio::load_audio_source(test_file).expect("Failed to load MIDI");
    let duration = source.get_duration_seconds();
    assert!(
        duration > 10.0,
        "Expected positive MIDI duration > 10s, got {}s",
        duration
    );
    let tempo = source.get_tempo();
    assert!(
        (80..=200).contains(&tempo),
        "Expected valid BPM in 80..=200, got {}",
        tempo
    );

    // Seek into the middle of the track and verify row advances
    source.set_position_seconds(20.0);
    let row_at_20s = source.get_current_row();
    assert!(
        row_at_20s > 0,
        "Expected MIDI tracker row > 0 at 20.0s, got {}",
        row_at_20s
    );
}

#[test]
fn test_audio_transition_and_fallback() {
    let mod_path = "audio_tests/ive_got_the_power.mod";
    let mid_path = "audio_tests/darude-sandstorm.mid";
    assert!(std::path::Path::new(mod_path).exists());
    assert!(std::path::Path::new(mid_path).exists());

    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));

    let handle1 = audio::start_audio_thread(mod_path, false, shared_state.clone())
        .expect("Failed to start first audio thread (MOD)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(handle1);

    std::thread::sleep(std::time::Duration::from_millis(100));

    let handle2 = audio::start_audio_thread(mid_path, false, shared_state.clone())
        .expect("Failed to start second audio track (MIDI)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(handle2);
}

#[test]
fn test_video_transition_and_fallback() {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));
    let video_path = "audio_tests/portrait_test_90.mp4";
    assert!(
        std::path::Path::new(video_path).exists(),
        "Expected fixture {} to exist",
        video_path
    );

    let handle = audio::start_audio_thread(video_path, false, shared_state.clone())
        .expect("Failed to start video audio thread");
    std::thread::sleep(std::time::Duration::from_millis(300));
    {
        let st = shared_state.lock().unwrap();
        assert!(
            st.has_video_stream,
            "Expected has_video_stream to be true for {}",
            video_path
        );
    }
    drop(handle);
}

#[test]
fn test_synthwave_lyrics_visualization_with_audio() {
    // 1. Verify Visualizer 23 definition
    let vis_def = state::VISUALIZERS
        .iter()
        .find(|v| v.id == 23)
        .expect("Visualizer ID 23 (3D Glass Water Lyrics) must exist");
    assert_eq!(vis_def.name, "3D Glass Water Lyrics");
    assert_eq!(vis_def.filename, "vis_lyrics.wgsl");

    // 2. Test sidecar discovery and loading with repo-local fixture
    let track_path = "audio_tests/arth_mb.mp3";
    assert!(
        std::path::Path::new(track_path).exists(),
        "Expected fixture {} to exist",
        track_path
    );
    let lrc = lyrics::load_lyrics_for_file(track_path)
        .expect("Expected sidecar lyrics audio_tests/arth_mb.lrc to load");
    assert!(!lrc.lines.is_empty(), "Parsed lyrics should have lines");

    let first_ts = lrc.lines[0].time_seconds;
    if first_ts > 0.5 {
        let intro_idx = lrc.find_current_line_idx(first_ts - 0.2);
        assert_eq!(
            intro_idx, None,
            "Timestamp before first line should return None (Intro)"
        );
    }

    let active_idx = lrc.find_current_line_idx(first_ts + 0.05);
    assert_eq!(
        active_idx,
        Some(0),
        "Timestamp at start of line 0 should return Some(0)"
    );

    if lrc.lines.len() > 1 {
        let second_ts = lrc.lines[1].time_seconds;
        let second_idx = lrc.find_current_line_idx(second_ts + 0.05);
        assert_eq!(
            second_idx,
            Some(1),
            "Timestamp at start of line 1 should return Some(1)"
        );
    }
}
