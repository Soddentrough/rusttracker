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
    let rx = {
        let st = shared_state.lock().unwrap();
        assert!(
            st.has_video_stream,
            "Expected has_video_stream to be true for {}",
            video_path
        );
        st.video_frame_rx
            .clone()
            .expect("Expected video_frame_rx to be populated for video file")
    };
    let frame = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("Expected at least one decoded VideoFrame from video decoder thread");
    assert!(
        frame.width > 0 && frame.height > 0 && !frame.y_plane.is_empty(),
        "Expected valid decoded video frame dimensions and Y plane"
    );
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

#[test]
fn test_all_audio_test_files() {
    let mut files: Vec<String> = Vec::new();
    for root_file in ["test_sine.wav", "test_multichannel_sweep.wav"] {
        if std::path::Path::new(root_file).exists() {
            files.push(root_file.to_string());
        }
    }
    if let Ok(entries) = std::fs::read_dir("audio_tests") {
        let mut dir_files: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if !p.is_file() {
                    return None;
                }
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                if len == 0 {
                    return None;
                }
                let ext = p
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext == "lrc" || ext == "txt" || ext == "json" {
                    return None;
                }
                Some(p.to_string_lossy().into_owned())
            })
            .collect();
        dir_files.sort();
        files.extend(dir_files);
    }

    println!("Testing {} sample files...", files.len());
    let mut failures = Vec::new();

    for path in &files {
        println!("\n=== Testing: {} ===", path);
        let load_res = std::panic::catch_unwind(|| audio::load_audio_source(path));
        match load_res {
            Err(panic_err) => {
                let msg = format!("{}: PANIC in load_audio_source: {:?}", path, panic_err);
                println!("  FAIL: {}", msg);
                failures.push(msg);
                continue;
            }
            Ok(Err(e)) => {
                let msg = format!("{}: load_audio_source error: {:?}", path, e);
                println!("  FAIL: {}", msg);
                failures.push(msg);
                continue;
            }
            Ok(Ok(mut src)) => {
                let ch = src.get_num_channels().max(1) as usize;
                let rate = src.get_intrinsic_sample_rate().unwrap_or(48000);
                let dur = src.get_duration_seconds();
                let mod_type = src.get_type();
                let has_vid = src.has_video_stream();
                println!(
                    "  Loaded: type={}, ch={}, rate={}Hz, dur={:.2}s, has_video={}",
                    mod_type, ch, rate, dur, has_vid
                );

                // Read up to 40 chunks (stereo downmix AND native channels)
                let read_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut buf = vec![0.0f32; 1024 * 2];
                    let mut total_frames = 0usize;
                    let mut max_abs = 0.0f32;
                    let mut max_vu = 0.0f32;
                    let vu_channels = src.get_tracker_channels().unwrap_or(ch as i32);
                    for _ in 0..40 {
                        let n = src.read_frames(2, rate, &mut buf);
                        if n == 0 {
                            break;
                        }
                        total_frames += n;
                        for &s in &buf[..n * 2] {
                            max_abs = max_abs.max(s.abs());
                        }
                        for c in 0..vu_channels {
                            max_vu = max_vu.max(src.get_current_channel_vu_mono(c));
                        }
                    }
                    // If intro is quiet (e.g. movie trailer / MIDI intro), also probe at 5.0s
                    if max_abs < 0.001 && dur > 6.0 {
                        src.set_position_seconds(5.0);
                        for _ in 0..40 {
                            let n = src.read_frames(2, rate, &mut buf);
                            if n == 0 {
                                break;
                            }
                            total_frames += n;
                            for &s in &buf[..n * 2] {
                                max_abs = max_abs.max(s.abs());
                            }
                            for c in 0..vu_channels {
                                max_vu = max_vu.max(src.get_current_channel_vu_mono(c));
                            }
                        }
                    }
                    let pos = src.get_position_seconds();
                    (total_frames, max_abs, max_vu, pos)
                }));

                match read_res {
                    Err(panic_err) => {
                        let msg = format!("{}: PANIC in read_frames(2ch): {:?}", path, panic_err);
                        println!("  FAIL: {}", msg);
                        failures.push(msg);
                    }
                    Ok((total_frames, max_abs, max_vu, pos)) => {
                        println!(
                            "  read_frames(2ch): frames={}, max_abs={:.6}, max_vu={:.6}, pos={:.3}s",
                            total_frames, max_abs, max_vu, pos
                        );
                        if total_frames == 0 {
                            failures.push(format!("{}: read_frames returned 0 frames", path));
                        }
                        if !path.contains("portrait_test_90") && max_abs == 0.0 {
                            failures.push(format!(
                                "{}: decoded audio is completely silent (max_abs=0)",
                                path
                            ));
                        }
                        if !path.contains("portrait_test_90") && max_vu == 0.0 {
                            failures.push(format!(
                                "{}: channel VUs remained 0 during playback (max_vu=0)",
                                path
                            ));
                        }
                    }
                }

                // Also test start_audio_thread end-to-end playback pipeline!
                let shared_state =
                    std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(path.clone())));
                {
                    let mut st = shared_state.lock().unwrap();
                    st.is_paused = false;
                }
                match audio::start_audio_thread(path, false, shared_state.clone()) {
                    Err(e) => {
                        let msg = format!("{}: start_audio_thread failed: {:?}", path, e);
                        println!("  FAIL: {}", msg);
                        failures.push(msg);
                    }
                    Ok(handle) => {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        let (hw_ch, cur_rate, num_ch, cur_sec, mod_t, spec_max, vu_len, raw_ch_len) = {
                            let st = shared_state.lock().unwrap();
                            let sm = st.raw_spectrum_data.iter().copied().fold(0.0f32, f32::max);
                            (
                                st.hardware_channels,
                                st.current_sample_rate,
                                st.num_channels,
                                st.current_seconds,
                                st.module_type.clone(),
                                sm,
                                st.raw_channel_vus.len(),
                                st.raw_audio_channels.first().map(|v| v.len()).unwrap_or(0),
                            )
                        };
                        println!(
                            "  start_audio_thread: mod={}, num_ch={}, hw_ch={}, rate={}Hz, pos={:.3}s, spec_max={:.2}, vu_len={}, raw_ch0_len={}",
                            mod_t, num_ch, hw_ch, cur_rate, cur_sec, spec_max, vu_len, raw_ch_len
                        );
                        if raw_ch_len == 0 {
                            failures.push(format!(
                                "{}: state.raw_audio_channels[0] was empty during playback",
                                path
                            ));
                        }
                        drop(handle);
                    }
                }
            }
        }
    }

    if !failures.is_empty() {
        panic!("Sample file failures:\n  {}", failures.join("\n  "));
    }
}
