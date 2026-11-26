mod common;

use rusttracker::{audio, state};

fn verify_multitrack_source_discovery(path: &str, expected_codecs: &[&str]) {
    let mut source = audio::load_audio_source(path)
        .unwrap_or_else(|e| panic!("Failed to load multitrack media {path}: {e:?}"));

    let tracks = source.get_audio_tracks();
    assert_eq!(
        tracks.len(),
        expected_codecs.len(),
        "Expected {} audio tracks in {path}",
        expected_codecs.len()
    );
    for (idx, expected) in expected_codecs.iter().enumerate() {
        assert_eq!(
            tracks[idx].codec, *expected,
            "Track {idx} codec mismatch in {path}"
        );
    }

    let mut buffer = vec![0.0f32; 1024 * 2];
    for (idx, track) in tracks.iter().enumerate() {
        source
            .select_audio_track(idx)
            .unwrap_or_else(|e| panic!("Failed to switch to track {idx} in {path}: {e:?}"));
        assert_eq!(source.get_selected_audio_track(), idx);
        assert_eq!(source.get_type(), track.codec);

        let frames = source.read_frames(2, 48000, &mut buffer);
        assert!(
            frames > 0,
            "Failed to read audio frames after switching to track {idx} in {path}"
        );
    }
}

#[test]
fn test_multitrack_source_discovery_and_switching() {
    let dir = common::TempTestDir::new("multitrack_discovery");
    let synth_mkv = dir.path().join("synthetic_multitrack.mkv");
    common::encode_synthetic_multitrack_mkv(&synth_mkv, 2.0)
        .expect("encode synthetic multitrack MKV");
    verify_multitrack_source_discovery(synth_mkv.to_str().unwrap(), &["AC3", "E-AC3", "AAC"]);

    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if std::path::Path::new(test_file).exists() {
        verify_multitrack_source_discovery(test_file, &["TrueHD", "AC3", "E-AC3", "AC3"]);
    }
}

fn verify_multitrack_playback_switching_via_state(
    path: &str,
    expected_tracks: usize,
    first_switch: usize,
    first_codec: &str,
    second_switch: usize,
    second_codec: &str,
) {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));

    let _handle = audio::start_audio_thread(path, false, shared_state.clone())
        .unwrap_or_else(|e| panic!("Failed to start audio thread for {path}: {e:?}"));

    let mut populated = false;
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.audio_tracks.len() == expected_tracks {
            populated = true;
            break;
        }
    }
    assert!(
        populated,
        "State should have {expected_tracks} audio tracks for {path}"
    );

    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(first_switch);
    }

    let mut switched = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == first_switch {
            switched = true;
            assert_eq!(state.module_type, first_codec);
            break;
        }
    }
    assert!(
        switched,
        "Decoder thread did not switch to audio track {first_switch} in time"
    );

    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(second_switch);
    }

    let mut switched_back = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == second_switch {
            switched_back = true;
            assert_eq!(state.module_type, second_codec);
            break;
        }
    }
    assert!(
        switched_back,
        "Decoder thread did not switch back to audio track {second_switch} in time"
    );
}

#[test]
fn test_multitrack_playback_switching_via_state() {
    let dir = common::TempTestDir::new("multitrack_state");
    let synth_mkv = dir.path().join("synthetic_multitrack.mkv");
    common::encode_synthetic_multitrack_mkv(&synth_mkv, 5.0)
        .expect("encode synthetic multitrack MKV");
    verify_multitrack_playback_switching_via_state(
        synth_mkv.to_str().unwrap(),
        3,
        1,
        "E-AC3",
        0,
        "AC3",
    );

    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if std::path::Path::new(test_file).exists() {
        verify_multitrack_playback_switching_via_state(test_file, 4, 2, "E-AC3", 0, "TrueHD");
    }
}

fn verify_singletrack_no_op(path: &str) {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));

    let _handle = audio::start_audio_thread(path, false, shared_state.clone())
        .expect("Failed to start audio thread");

    let mut ready = false;
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if !state.audio_tracks.is_empty() {
            ready = true;
            assert_eq!(
                state.audio_tracks.len(),
                1,
                "Single-stream file should have exactly 1 track"
            );
            assert_eq!(state.selected_audio_track, 0);
            break;
        }
    }
    assert!(
        ready,
        "Timed out waiting for singletrack metadata on {path}"
    );

    let initial_epoch = {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(0);
        state.seek_epoch
    };

    std::thread::sleep(std::time::Duration::from_millis(120));

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(
            state.seek_epoch, initial_epoch,
            "seek_epoch should not increment on redundant/single-track switch"
        );
    }
}

#[test]
fn test_singletrack_no_op_and_position_preservation() {
    let dir = common::TempTestDir::new("singletrack_noop");
    let synth_flac = dir.path().join("synthetic_51.flac");
    common::encode_synthetic_flac(&synth_flac, 6, 48_000, 3.0).expect("encode synthetic 5.1 FLAC");
    verify_singletrack_no_op(synth_flac.to_str().unwrap());

    let test_file = "audio_tests/FLAC 5.1.flac";
    if std::path::Path::new(test_file).exists() {
        verify_singletrack_no_op(test_file);
    }
}

fn verify_multitrack_mixing(path: &str) {
    let mut source =
        audio::load_audio_source(path).unwrap_or_else(|e| panic!("Failed to load {path}: {e:?}"));

    let tracks = source.get_audio_tracks();
    assert!(tracks.len() >= 2, "Test requires at least 2 tracks");

    source
        .set_active_audio_tracks(&[(0, 1.0), (1, 1.0)])
        .expect("Failed to set active audio tracks");

    let active_tracks = source.get_active_audio_tracks();
    assert_eq!(active_tracks.len(), 2, "Should have 2 active tracks");
    assert!(active_tracks.contains(&0));
    assert!(active_tracks.contains(&1));

    let mut buffer = vec![0.0f32; 1024 * 2];
    let frames = source.read_frames(2, 48000, &mut buffer);
    assert!(frames > 0, "Failed to read frames from mixed streams");

    source
        .set_active_audio_tracks(&[(0, 1.0), (1, 0.0)])
        .expect("Failed to adjust track volumes");

    let frames_after_adjust = source.read_frames(2, 48000, &mut buffer);
    assert!(
        frames_after_adjust > 0,
        "Failed to read frames after volume adjustment"
    );
}

#[test]
fn test_multitrack_mixing_and_volume_blending() {
    let dir = common::TempTestDir::new("multitrack_mix");
    let synth_mkv = dir.path().join("synthetic_multitrack.mkv");
    common::encode_synthetic_multitrack_mkv(&synth_mkv, 2.0)
        .expect("encode synthetic multitrack MKV");
    verify_multitrack_mixing(synth_mkv.to_str().unwrap());

    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if std::path::Path::new(test_file).exists() {
        verify_multitrack_mixing(test_file);
    }
}

fn verify_multitrack_waveform_timeline_on_track_switch(path: &str, switch_track: usize) {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new(
        "Test App".to_string(),
    )));
    {
        let mut state = shared_state.lock().unwrap();
        state.is_paused = false;
    }

    let _handle = audio::start_audio_thread(path, false, shared_state.clone())
        .unwrap_or_else(|e| panic!("Failed to start audio thread for {path}: {e:?}"));

    let mut populated = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        let max_amp = state
            .lookahead_timeline
            .iter()
            .fold(0.0f32, |acc, &v| acc.max(v.abs()));
        if !state.lookahead_queue.is_empty() && max_amp > 0.0 {
            populated = true;
            break;
        }
    }
    assert!(
        populated,
        "Lookahead queue/timeline must populate initially for {path}"
    );

    {
        let state = shared_state.lock().unwrap();
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
    }

    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(switch_track);
    }

    let mut switched = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == switch_track {
            switched = true;
            break;
        }
    }
    assert!(switched, "Failed to switch to track {switch_track}");

    std::thread::sleep(std::time::Duration::from_millis(350));
    {
        let state = shared_state.lock().unwrap();
        assert!(
            !state.lookahead_queue.is_empty(),
            "Lookahead queue should be populated on switched track"
        );
        let mut prev_t = -1.0;
        for (t, _) in state.lookahead_queue.iter() {
            assert!(
                *t > prev_t,
                "Lookahead queue timestamps must be strictly monotonic on track {switch_track}: {} <= {}",
                *t,
                prev_t
            );
            prev_t = *t;
        }
    }

    {
        let mut state = shared_state.lock().unwrap();
        state.audio_track_request = Some(0);
    }

    let mut switched_back = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        if state.selected_audio_track == 0 {
            switched_back = true;
            break;
        }
    }
    assert!(switched_back, "Failed to switch back to track 0");

    let mut back_populated = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let state = shared_state.lock().unwrap();
        let max_amp = state
            .lookahead_timeline
            .iter()
            .fold(0.0f32, |acc, &v| acc.max(v.abs()));
        if !state.lookahead_queue.is_empty() && max_amp > 0.0 {
            back_populated = true;
            let mut prev_t = -1.0;
            for (t, _) in state.lookahead_queue.iter() {
                assert!(
                    *t > prev_t,
                    "Lookahead queue timestamps must be strictly monotonic after switch-back: {} <= {}",
                    *t,
                    prev_t
                );
                prev_t = *t;
            }
            break;
        }
    }
    assert!(
        back_populated,
        "Timeline must contain non-zero audio waveform data after switch-back"
    );
}

#[test]
fn test_multitrack_waveform_timeline_on_track_switch() {
    let dir = common::TempTestDir::new("multitrack_timeline");
    let synth_mkv = dir.path().join("synthetic_multitrack.mkv");
    common::encode_synthetic_multitrack_mkv(&synth_mkv, 6.0)
        .expect("encode synthetic multitrack MKV");
    verify_multitrack_waveform_timeline_on_track_switch(synth_mkv.to_str().unwrap(), 1);

    let test_file = "audio_tests/Dolby Atmos TrueHD, E-AC-3 7.1.4.mkv";
    if std::path::Path::new(test_file).exists() {
        verify_multitrack_waveform_timeline_on_track_switch(test_file, 2);
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
    let expected_m4 = 0.35 * (0.5 + 0.1 + std::f32::consts::FRAC_1_SQRT_2 * (0.4 + 0.1));
    assert!(
        (m4 - expected_m4).abs() < 1e-4,
        "Expected exact ITU-R BS.775-3 quad mono folddown"
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

fn verify_symphonia_7_1_flac_decoding_and_seeking(path: &str) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("Failed to open {path}: {e}"));
    let mut sym = audio::try_symphonia(file, "flac", "flac", None, Some(path))
        .unwrap_or_else(|e| panic!("Symphonia must decode 7.1 FLAC {path}: {e}"));

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

#[test]
fn test_symphonia_7_1_flac_decoding_and_seeking() {
    let dir = common::TempTestDir::new("symphonia_71_flac");
    let synth_flac = dir.path().join("synthetic_71.flac");
    common::encode_synthetic_flac(&synth_flac, 8, 48_000, 2.5).expect("encode synthetic 7.1 FLAC");
    verify_symphonia_7_1_flac_decoding_and_seeking(synth_flac.to_str().unwrap());

    let test_file = "audio_tests/FLAC 7.1.flac";
    if std::path::Path::new(test_file).exists() {
        verify_symphonia_7_1_flac_decoding_and_seeking(test_file);
    }
}
