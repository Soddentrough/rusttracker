#[path = "../src/audio.rs"]
pub mod audio;
#[path = "../src/bitstream.rs"]
pub mod bitstream;
#[path = "../src/lyrics.rs"]
pub mod lyrics;
#[path = "../src/state.rs"]
pub mod state;

#[test]
fn test_midi_loading() {
    let test_file = "audio_tests/darude-sandstorm.mid";
    if !std::path::Path::new(test_file).exists() {
        println!("Skipping MIDI test: fixture {} does not exist", test_file);
        return;
    }
    let result = audio::load_audio_source(test_file);
    assert!(result.is_ok(), "Failed to load MIDI: {:?}", result.err());
    let mut source = result.unwrap();
    println!("Parsed MIDI duration: {}s", source.get_duration_seconds());
}

#[test]
fn test_audio_transition_and_fallback() {
    let mod_path = "audio_tests/ive_got_the_power.mod";
    let mid_path = "audio_tests/darude-sandstorm.mid";
    if !std::path::Path::new(mod_path).exists() || !std::path::Path::new(mid_path).exists() {
        println!(
            "Skipping audio transition test: fixtures {} or {} do not exist",
            mod_path, mid_path
        );
        return;
    }

    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));

    println!("Starting first audio track (MOD)...");
    let handle1 = audio::start_audio_thread(mod_path, false, shared_state.clone());
    assert!(
        handle1.is_ok(),
        "Failed to start first audio thread: {:?}",
        handle1.err()
    );
    let handle1 = handle1.unwrap();

    std::thread::sleep(std::time::Duration::from_secs(1));

    println!("Stopping first audio track (dropping handle)...");
    drop(handle1);

    std::thread::sleep(std::time::Duration::from_millis(200));

    println!("Starting second audio track (MIDI transition)...");
    let handle2 = audio::start_audio_thread(mid_path, false, shared_state.clone());
    assert!(
        handle2.is_ok(),
        "Failed to start second audio track: {:?}",
        handle2.err()
    );
    let handle2 = handle2.unwrap();

    std::thread::sleep(std::time::Duration::from_secs(1));

    println!("Stopping second audio track (dropping handle)...");
    drop(handle2);

    println!("Audio transition test completed successfully!");
}

#[test]
fn test_video_transition_and_fallback() {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));
    let movie_path = "/home/naoki/Music/Jeff Wayne's Musical Version Of The War Of The Worlds The New Generation (2013) [1080p] [BluRay] [5.1] [YTS.MX]/Jeff.Wayne's.Musical.Version.Of.The.War.Of.The.Worlds.The.New.Generation.2013.1080p.BluRay.x264.AAC5.1-[YTS.MX].mp4";

    if std::path::Path::new(movie_path).exists() {
        println!("Starting video audio track...");
        let handle = audio::start_audio_thread(movie_path, false, shared_state.clone());
        assert!(
            handle.is_ok(),
            "Failed to start video audio thread: {:?}",
            handle.err()
        );
        let handle = handle.unwrap();

        std::thread::sleep(std::time::Duration::from_secs(1));

        println!("Stopping video audio track...");
        drop(handle);
    } else {
        println!("Movie file not found, skipping video transition test.");
    }
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

    // 2. Test sidecar discovery and loading with real karaoke packages & test tracks
    let sample_tracks = [
        "audio_tests/arth_mb.mp3",
        "test_sine.wav",
        "/home/naoki/src/ComfyUI/output/karaoke_package/Cake - I Will Survive.flac",
        "/home/naoki/src/ComfyUI/output/karaoke_package/Depeche Mode - Enjoy the Silence.flac",
        "/home/naoki/src/ComfyUI/output/karaoke_package/Tom Odell - Another Love.flac",
    ];

    let mut found_and_tested = 0;
    for track_path in sample_tracks {
        if std::path::Path::new(track_path).exists() {
            let loaded = lyrics::load_lyrics_for_file(track_path);
            if loaded.is_none() {
                continue;
            }
            let lrc = loaded.unwrap();
            assert!(!lrc.lines.is_empty(), "Parsed lyrics should have lines");

            // Test intro phase (before first line)
            let first_ts = lrc.lines[0].time_seconds;
            if first_ts > 0.5 {
                let intro_idx = lrc.find_current_line_idx(first_ts - 0.2);
                assert_eq!(
                    intro_idx, None,
                    "Timestamp before first line should return None (Intro)"
                );
            }

            // Test active verse phase
            let active_idx = lrc.find_current_line_idx(first_ts + 0.05);
            assert_eq!(
                active_idx,
                Some(0),
                "Timestamp at start of line 0 should return Some(0)"
            );

            // Test progression to subsequent line
            if lrc.lines.len() > 1 {
                let second_ts = lrc.lines[1].time_seconds;
                let second_idx = lrc.find_current_line_idx(second_ts + 0.05);
                assert_eq!(
                    second_idx,
                    Some(1),
                    "Timestamp at start of line 1 should return Some(1)"
                );
            }

            println!(
                "Verified Synthwave Lyrics handling for {}: {} lines loaded (First line at {:.2}s: '{}')",
                track_path,
                lrc.lines.len(),
                first_ts,
                lrc.lines[0].text
            );
            found_and_tested += 1;
        }
    }

    if found_and_tested == 0 {
        println!("Skipping lyrics test: no test tracks with lyrics sidecars found.");
    }
}
