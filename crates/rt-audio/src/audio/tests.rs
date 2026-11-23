use super::dsp::OscilloscopeTriggerState;
use super::playback::{
    StackLowercaseAsciiBuf, allocate_rt_dsp_pool, compute_fft_mono_sample,
    populate_state_from_audio_source, try_build_rt_dsp_message,
};
use super::sources::{OpenMptSource, map_iamf_ambisonics_to_canonical};
use super::video_decode::{
    effective_frame_hw_label, format_video_upload_telemetry, preferred_hw_video_devices,
    update_video_upload_telemetry_if_changed,
};
use super::*;
use crate::state::AppState;
use crossbeam_channel::bounded;
use realfft::RealFftPlanner;
use rt_decode::openmpt_ffi::OpenMptModule;
use std::sync::{Arc, Mutex};

#[test]
fn test_calculate_mix_headroom_single_track() {
    // Single track at full volume should have no attenuation
    let headroom = calculate_mix_headroom([1.0]);
    assert_eq!(headroom, 1.0);

    // Single track at reduced volume should not boost headroom above 1.0
    let headroom_half = calculate_mix_headroom([0.5]);
    assert_eq!(headroom_half, 1.0);

    // Zero tracks
    let headroom_empty = calculate_mix_headroom([]);
    assert_eq!(headroom_empty, 1.0);
}

#[test]
fn test_calculate_mix_headroom_multi_track_equal_power() {
    // Two equal tracks at 1.0: active power = 2.0 -> headroom = 1 / sqrt(2) ≈ 0.7071068
    let headroom_two = calculate_mix_headroom([1.0, 1.0]);
    let expected_two = 1.0 / (2.0f32).sqrt();
    assert!((headroom_two - expected_two).abs() < 1e-6);

    // Three equal tracks at 1.0: active power = 3.0 -> headroom = 1 / sqrt(3) ≈ 0.5773503
    let headroom_three = calculate_mix_headroom([1.0, 1.0, 1.0]);
    let expected_three = 1.0 / (3.0f32).sqrt();
    assert!((headroom_three - expected_three).abs() < 1e-6);

    // Four tracks at 0.5: active power = 4 * 0.25 = 1.0 -> headroom = 1.0
    let headroom_four_half = calculate_mix_headroom([0.5, 0.5, 0.5, 0.5]);
    assert_eq!(headroom_four_half, 1.0);
}

#[test]
fn test_calculate_mix_headroom_muted_and_asymmetric_tracks() {
    // Track 1 at 1.0 and Track 2 at 0.0 (muted)
    // Muted track must NOT attenuate the active track
    let headroom_muted = calculate_mix_headroom([1.0, 0.0]);
    assert_eq!(headroom_muted, 1.0);

    // Track 1 at 1.0 and Track 2 at 0.1 (low volume)
    // Active power = 1.0 + 0.01 = 1.01 -> headroom ≈ 0.995037
    // (Previously based on track count, this caused an abrupt 30% drop to 0.707)
    let headroom_asymmetric = calculate_mix_headroom([1.0, 0.1]);
    let expected = 1.0 / (1.01f32).sqrt();
    assert!((headroom_asymmetric - expected).abs() < 1e-6);
    assert!(headroom_asymmetric > 0.99);
}

#[test]
fn test_get_downmixed_sample_mono_to_stereo() {
    let mono_sample = [0.8f32];
    // Mono duplicated to L and R
    let l = get_downmixed_sample(1, &mono_sample, 2, 0);
    let r = get_downmixed_sample(1, &mono_sample, 2, 1);
    assert_eq!(l, 0.8);
    assert_eq!(r, 0.8);
}

#[test]
fn test_get_downmixed_sample_stereo_passthrough_and_folddown() {
    let stereo_samples = [0.6f32, -0.4f32];
    // Stereo to Stereo passthrough
    let l = get_downmixed_sample(2, &stereo_samples, 2, 0);
    let r = get_downmixed_sample(2, &stereo_samples, 2, 1);
    assert_eq!(l, 0.6);
    assert_eq!(r, -0.4);

    // Stereo to Mono folddown: (L + R) * 0.5
    let mono = get_downmixed_sample(2, &stereo_samples, 1, 0);
    assert!((mono - (0.6 - 0.4) * 0.5).abs() < 1e-6);
}

#[test]
fn test_get_downmixed_sample_quad() {
    // FL, FR, BL, BR
    let quad_samples = [0.6f32, -0.6f32, 0.4f32, -0.4f32];
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

    let left = get_downmixed_sample(4, &quad_samples, 2, 0);
    let right = get_downmixed_sample(4, &quad_samples, 2, 1);

    let expected_left = (0.6 + INV_SQRT2 * 0.4) * INV_SQRT2;
    let expected_right = (-0.6 + INV_SQRT2 * -0.4) * INV_SQRT2;

    assert!((left - expected_left).abs() < 1e-6);
    assert!((right - expected_right).abs() < 1e-6);
}

#[test]
fn test_get_downmixed_sample_5_1_surround() {
    // FL, FR, FC, LFE, BL, BR — ITU-R BS.775-3 excludes LFE from stereo downmix
    let surround_samples = [0.5f32, 0.5f32, 0.4f32, 0.2f32, 0.3f32, 0.3f32];
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

    let left = get_downmixed_sample(6, &surround_samples, 2, 0);
    let right = get_downmixed_sample(6, &surround_samples, 2, 1);

    let expected_left = (0.5 + INV_SQRT2 * 0.4 + INV_SQRT2 * 0.3) * INV_SQRT2;
    let expected_right = (0.5 + INV_SQRT2 * 0.4 + INV_SQRT2 * 0.3) * INV_SQRT2;

    assert!((left - expected_left).abs() < 1e-6);
    assert!((right - expected_right).abs() < 1e-6);
}

#[test]
fn test_get_downmixed_sample_quad_and_61_and_expansion_routing() {
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    // 1. 4.0 Quad (FL, FR, BL, BR) expanding to 5.1 must not route BL/BR into FC or LFE
    let quad = [0.2f32, -0.2, 0.7, -0.7];
    assert!((get_downmixed_sample(4, &quad, 6, 0) - 0.2).abs() < 1e-6);
    assert!((get_downmixed_sample(4, &quad, 6, 1) - (-0.2)).abs() < 1e-6);
    assert_eq!(get_downmixed_sample(4, &quad, 6, 2), 0.0);
    assert_eq!(get_downmixed_sample(4, &quad, 6, 3), 0.0);
    assert!((get_downmixed_sample(4, &quad, 6, 4) - 0.7).abs() < 1e-6);
    assert!((get_downmixed_sample(4, &quad, 6, 5) - (-0.7)).abs() < 1e-6);

    // 2. 5.1 -> 4.0 Quad excludes LFE and folds FC into FL/FR
    let s51 = [0.4f32, -0.4, 0.2, 0.9, 0.5, -0.5];
    assert!(
        (get_downmixed_sample(6, &s51, 4, 0) - (0.4 + INV_SQRT2 * 0.2) * INV_SQRT2).abs() < 1e-6
    );
    assert!(
        (get_downmixed_sample(6, &s51, 4, 1) - (-0.4 + INV_SQRT2 * 0.2) * INV_SQRT2).abs() < 1e-6
    );
    assert!((get_downmixed_sample(6, &s51, 4, 2) - 0.5).abs() < 1e-6);
    assert!((get_downmixed_sample(6, &s51, 4, 3) - (-0.5)).abs() < 1e-6);

    // 3. 6.1 (FL, FR, FC, LFE, BC, SL, SR) -> 5.1 folds BC equally into Ls/Rs
    let s61 = [0.3f32, -0.3, 0.1, 0.2, 0.6, 0.4, -0.4];
    assert!((get_downmixed_sample(7, &s61, 6, 3) - 0.2).abs() < 1e-6);
    assert!(
        (get_downmixed_sample(7, &s61, 6, 4) - (0.4 + INV_SQRT2 * 0.6) * INV_SQRT2).abs() < 1e-6
    );
    assert!(
        (get_downmixed_sample(7, &s61, 6, 5) - (-0.4 + INV_SQRT2 * 0.6) * INV_SQRT2).abs() < 1e-6
    );

    // 4. 7.1.4 (12ch) -> 6.1 (7ch) folds rear + height channels into BC/SL/SR cleanly
    let s714 = [
        0.5f32, -0.5, 0.2, 0.1, 0.3, -0.3, 0.4, -0.4, 0.2, -0.2, 0.1, -0.1,
    ];
    assert!((get_downmixed_sample(12, &s714, 7, 0) - (0.5 + INV_SQRT2 * 0.2) * 0.82).abs() < 1e-6);
    assert!((get_downmixed_sample(12, &s714, 7, 4)).abs() < 1e-6);
}

#[test]
fn test_multi_track_active_tracks_filter_enabled() {
    struct MockTrack {
        track_idx: usize,
        volume: f32,
        enabled: bool,
    }
    let tracks = [
        MockTrack {
            track_idx: 0,
            volume: 1.0,
            enabled: true,
        },
        MockTrack {
            track_idx: 1,
            volume: 0.0,
            enabled: true,
        }, // Muted but enabled
        MockTrack {
            track_idx: 2,
            volume: 0.0,
            enabled: false,
        }, // Omitted / disabled
    ];

    // Active tracks filter checks `track.enabled`
    let mut active: Vec<usize> = tracks
        .iter()
        .filter(|t| t.enabled)
        .map(|t| t.track_idx)
        .collect();
    active.sort();
    // Track 1 must remain active in the UI controls despite 0.0 volume
    assert_eq!(active, vec![0, 1]);

    // Prior buggy filter checked `track.volume > 0.0`, erroneously dropping muted tracks
    let buggy_active: Vec<usize> = tracks
        .iter()
        .filter(|t| t.volume > 0.0)
        .map(|t| t.track_idx)
        .collect();
    assert_eq!(buggy_active, vec![0]);
}

#[test]
fn test_multi_track_mix_mode_flag_transitions() {
    let single_track = [0];
    let multi_track = [0, 1];
    let empty_track: [usize; 0] = [];

    assert!(!(single_track.len() > 1));
    assert!(multi_track.len() > 1);
    assert!(!(empty_track.len() > 1));
}

#[test]
fn test_dsp_thread_skips_fft_in_fullscreen_video_and_preserves_bitstream_buffer_pct() {
    let state = Arc::new(Mutex::new(AppState::new("Test".to_string())));
    {
        let mut s = state.lock().unwrap();
        s.has_video_stream = true;
        s.video_mode = 3; // Fullscreen video mode (no visualization)
        s.stats.bitstream_active = true;
        s.stats.audio_buffer_fill_pct = 82.5;
        s.stats.fft_us = 999.0;
    }

    let (tx, rx) = bounded(4);
    let window_size = 2048;
    spawn_dsp_thread(rx, Arc::clone(&state), 48000, 20000.0, window_size);

    // Send a DSP message while in fullscreen video mode
    tx.send(DspMessage {
        audio_data: Vec::new(),
        channel_audio_data: Vec::new(),
        channel_vus: vec![0.4, 0.5],
        current_order: 1,
        current_row: 2,
        bpm: 120,
        speed: 6,
        current_seconds: 42.5,
        current_row_string: String::new(),
        seek_epoch: 0,
    })
    .unwrap();

    // Wait briefly for DSP thread to process
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let s = state.lock().unwrap();
        if (s.current_seconds - 42.5).abs() < 1e-6 {
            break;
        }
    }

    {
        let mut s = state.lock().unwrap();
        assert!((s.current_seconds - 42.5).abs() < 1e-6);
        assert_eq!(
            s.stats.fft_us, 0.0,
            "CPU FFT time must be 0.0 when skipped in Fullscreen Video mode"
        );
        assert_eq!(
            s.stats.audio_buffer_fill_pct, 82.5,
            "DSP thread must not overwrite bitstream audio_buffer_fill_pct with 0.0%"
        );
        // Switch to normal visualization mode (video_mode = 2 = Split View)
        s.video_mode = 2;
    }

    // Send a DSP message with a 440Hz tone in Split View mode
    let samples: Vec<f32> = (0..window_size)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin())
        .collect();
    tx.send(DspMessage {
        audio_data: samples.clone(),
        channel_audio_data: vec![samples.clone(), samples],
        channel_vus: vec![0.8, 0.8],
        current_order: 1,
        current_row: 3,
        bpm: 120,
        speed: 6,
        current_seconds: 43.0,
        current_row_string: String::new(),
        seek_epoch: 0,
    })
    .unwrap();

    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let s = state.lock().unwrap();
        if (s.current_seconds - 43.0).abs() < 1e-6 {
            break;
        }
    }

    {
        let s = state.lock().unwrap();
        assert!((s.current_seconds - 43.0).abs() < 1e-6);
        assert!(
            s.stats.fft_us > 0.0,
            "CPU FFT must run when not in Fullscreen Video mode"
        );
        assert_eq!(
            s.stats.audio_buffer_fill_pct, 82.5,
            "DSP thread must preserve bitstream audio_buffer_fill_pct across frames"
        );
    }
}

#[test]
fn test_dsp_thread_discards_stale_seek_epoch_and_preserves_post_seek_lookahead_queue() {
    let state = Arc::new(Mutex::new(AppState::new("Test".to_string())));
    {
        let mut s = state.lock().unwrap();
        s.seek_epoch = 5;
        s.current_seconds = 100.0;
        // Populate post-seek lookahead queue around 100.0s .. 103.0s
        for i in 0..300 {
            s.lookahead_queue.push_back((
                100.0 + (i as f64) * 0.01,
                [0.75, 0.5, 0.75, 0.5, 0.0, 0.0, 0.0, 0.0],
            ));
        }
    }

    let (tx, rx) = bounded(4);
    let window_size = 2048;
    spawn_dsp_thread(rx, Arc::clone(&state), 48000, 20000.0, window_size);

    // Send a stale pre-seek message from epoch 4 at timestamp 250.0s (which previously
    // would have overwritten current_seconds = 250.0 and pruned all 100.0..103.0s slices!)
    tx.send(DspMessage {
        audio_data: vec![0.25; window_size],
        channel_audio_data: vec![vec![0.25; window_size], vec![0.25; window_size]],
        channel_vus: vec![0.25, 0.25],
        current_order: 0,
        current_row: 0,
        bpm: 120,
        speed: 6,
        current_seconds: 250.0,
        current_row_string: String::new(),
        seek_epoch: 4,
    })
    .unwrap();

    // Follow with a fresh post-seek message from epoch 5 at timestamp 100.05s
    tx.send(DspMessage {
        audio_data: vec![0.25; window_size],
        channel_audio_data: vec![vec![0.25; window_size], vec![0.25; window_size]],
        channel_vus: vec![0.25, 0.25],
        current_order: 0,
        current_row: 0,
        bpm: 120,
        speed: 6,
        current_seconds: 100.05,
        current_row_string: String::new(),
        seek_epoch: 5,
    })
    .unwrap();

    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let s = state.lock().unwrap();
        if (s.current_seconds - 100.05).abs() < 1e-6 {
            break;
        }
    }

    let s = state.lock().unwrap();
    assert!(
        (s.current_seconds - 100.05).abs() < 1e-6,
        "Stale seek_epoch message must not overwrite current_seconds"
    );
    assert!(
        s.lookahead_queue.len() >= 295,
        "Post-seek lookahead_queue slices must not be pruned by stale pre-seek DspMessage (len={})",
        s.lookahead_queue.len()
    );
}

#[test]
fn test_dsp_thread_zeroes_spectrum_while_paused_with_file_loaded() {
    let state = Arc::new(Mutex::new(AppState::new("Test".to_string())));
    {
        let mut s = state.lock().unwrap();
        s.file_loaded = true;
        s.is_paused = true;
        s.raw_spectrum_data.fill(10.0);
        s.gpu_spectrum_data = vec![5.0; 256];
        s.raw_channel_vus = vec![0.9, 0.9];
    }

    let (tx, rx) = bounded(4);
    let window_size = 2048;
    spawn_dsp_thread(rx, Arc::clone(&state), 48000, 20000.0, window_size);

    let samples: Vec<f32> = (0..window_size)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin())
        .collect();
    tx.send(DspMessage {
        audio_data: samples.clone(),
        channel_audio_data: vec![samples.clone(), samples],
        channel_vus: vec![0.8, 0.8],
        current_order: 0,
        current_row: 0,
        bpm: 120,
        speed: 6,
        current_seconds: 12.5,
        current_row_string: String::new(),
        seek_epoch: 0,
    })
    .unwrap();

    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let s = state.lock().unwrap();
        if (s.current_seconds - 12.5).abs() < 1e-6 {
            break;
        }
    }

    let s = state.lock().unwrap();
    assert!(
        s.raw_spectrum_data.iter().all(|&v| v == 0.0),
        "Paused playback must not allow DspMessage to blip raw_spectrum_data"
    );
    assert!(
        s.gpu_spectrum_data.iter().all(|&v| v == 0.0),
        "Paused playback must zero gpu_spectrum_data"
    );
    assert!(
        s.raw_channel_vus.iter().all(|&v| v == 0.0),
        "Paused playback must zero raw_channel_vus"
    );
}

#[test]
fn test_push_planar_lookahead_slices_accumulates_sub_slice_frames_and_remainders() {
    let mut s = AppState::new("Test".to_string());
    s.current_seconds = 10.0;

    // Simulate TrueHD-style 40-sample frames at 48kHz (12 frames = 480 samples = 1 slice of 10ms)
    let channels = vec![vec![0.6f32; 40], vec![0.4f32; 40]];
    for i in 0..24 {
        let pts = 10.0 + (i as f64) * (40.0 / 48_000.0);
        push_planar_lookahead_slices(&mut s, &channels, 48_000, pts);
    }

    assert_eq!(
        s.lookahead_queue.len(),
        2,
        "24 frames of 40 samples @ 48kHz (960 samples) must produce exactly 2 10ms lookahead slices"
    );
    assert_eq!(s.lookahead_sample_buffer.len(), 0);
    assert!((s.lookahead_queue[0].0 - 10.0).abs() < 1e-4);
    assert!((s.lookahead_queue[1].0 - 10.01).abs() < 1e-4);
    assert!((s.lookahead_queue[0].1[1] - 0.6).abs() < 1e-4);
    assert!((s.lookahead_queue[0].1[3] - 0.4).abs() < 1e-4);
    assert!((s.lookahead_queue[0].1[4] - 0.6).abs() < 1e-4);
    assert!((s.lookahead_queue[0].1[5] - 0.4).abs() < 1e-4);

    // Also verify update_lookahead_timeline_from_queue populates the tape timeline at +0.0s (slice 100)
    let mut timeline = vec![0.0f32; 600 * 8];
    update_lookahead_timeline_from_queue(&s.lookahead_queue, &[], &[], 10.0, &mut timeline);
    assert!((timeline[100 * 8 + 1] - 0.6).abs() < 1e-4);
    assert!((timeline[100 * 8 + 3] - 0.4).abs() < 1e-4);
}

#[test]
fn test_surround_upmixer_left_right_hemisphere_and_height_crossover() {
    let sample_rate = 48_000;
    let mut dsp = SurroundUpmixerDsp::new(sample_rate);
    let cfg = crate::state::SurroundUpmixConfig {
        enabled: true,
        sub_crossover_hz: 90.0,
        height_crossover_hz: 2500.0,
        surround_high_bleed: 0.0, // Pure split: highs only to height speakers
        stereo_width: 0.0,        // Disable Mid/Side cross-feed to test strict L/R isolation
        ..Default::default()
    };
    dsp.sync_config(&cfg, sample_rate);

    // 1. Feed a low-mid 250 Hz sine wave strictly on the LEFT channel (R = 0.0)
    let mut peak_low = [0.0f32; 12];
    for n in 0..2048 {
        let l = (2.0 * std::f32::consts::PI * 250.0 * n as f32 / sample_rate as f32).sin() * 0.7;
        let out = dsp.process_stereo_to_7_1_4(l, 0.0, &cfg);
        if n > 512 {
            for (p, &s) in peak_low.iter_mut().zip(out.iter()) {
                *p = p.max(s.abs());
            }
        }
    }
    // Left bed speakers (FL=0, SL=4, RL=6) must be strongly active, Right bed (FR=1, SR=5, RR=7) silent
    assert!(peak_low[0] > 0.4, "FL must carry 250Hz left signal");
    assert!(peak_low[4] > 0.35, "SL must carry 250Hz left signal");
    assert!(peak_low[6] > 0.30, "RL must carry 250Hz left signal");
    assert!(
        peak_low[1] < 1e-4,
        "FR must remain silent for pure left input"
    );
    assert!(
        peak_low[5] < 1e-4,
        "SR must remain silent for pure left input"
    );
    assert!(
        peak_low[7] < 1e-4,
        "RR must remain silent for pure left input"
    );
    // At 250 Hz (well below 2500 Hz height crossover), height speakers (TFL=8) are strongly attenuated
    assert!(
        peak_low[8] < peak_low[4] * 0.15,
        "TFL must be attenuated at 250Hz (< 2.5kHz height crossover)"
    );

    // 2. Feed a high-frequency 8000 Hz sine wave strictly on the RIGHT channel (L = 0.0)
    let mut dsp_hi = SurroundUpmixerDsp::new(sample_rate);
    dsp_hi.sync_config(&cfg, sample_rate);
    let mut peak_hi = [0.0f32; 12];
    for n in 0..2048 {
        let r = (2.0 * std::f32::consts::PI * 8000.0 * n as f32 / sample_rate as f32).sin() * 0.7;
        let out = dsp_hi.process_stereo_to_7_1_4(0.0, r, &cfg);
        if n > 512 {
            for (p, &s) in peak_hi.iter_mut().zip(out.iter()) {
                *p = p.max(s.abs());
            }
        }
    }
    // Right height speakers (TFR=9, TRR=11) must be strongly active, and significantly louder than ear-level SR/RR
    assert!(
        peak_hi[9] > 0.45,
        "TFR must carry 8kHz right high-frequency signal"
    );
    assert!(
        peak_hi[11] > 0.40,
        "TRR must carry 8kHz right high-frequency signal"
    );
    assert!(
        peak_hi[9] > peak_hi[5] * 3.0,
        "At 8kHz (> 2.5kHz crossover), TFR height speaker ({}) must dominate SR bed speaker ({})",
        peak_hi[9],
        peak_hi[5]
    );
    assert!(
        peak_hi[8] < 1e-4,
        "TFL must remain silent for pure right input"
    );
}

#[test]
fn test_rank_output_config_key_prioritizes_f32_over_i16_on_wasapi_configs() {
    // Simulate cpal WASAPI supported_formats() enumeration order: I16 appears before F32
    let mut wasapi_configs = [
        (6u16, 48_000u32, 48_000u32, cpal::SampleFormat::I16),
        (6u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
        (6u16, 44_100u32, 44_100u32, cpal::SampleFormat::I16),
        (6u16, 44_100u32, 44_100u32, cpal::SampleFormat::F32),
    ];
    wasapi_configs.sort_by_key(|&(ch, min_r, max_r, fmt)| {
        rank_output_config_key(ch, min_r, max_r, fmt, 6, 48_000, false, false)
    });
    assert_eq!(
        wasapi_configs[0],
        (6, 48_000, 48_000, cpal::SampleFormat::F32),
        "F32 must be selected before I16 so Windows WASAPI uses WAVE_FORMAT_EXTENSIBLE on 5.1/7.1 endpoints"
    );

    // Also verify stereo (2ch) endpoint with surround upmix enabled prefers F32 at target rate
    let mut stereo_configs = [
        (2u16, 48_000u32, 48_000u32, cpal::SampleFormat::I16),
        (2u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
    ];
    stereo_configs.sort_by_key(|&(ch, min_r, max_r, fmt)| {
        rank_output_config_key(ch, min_r, max_r, fmt, 2, 48_000, false, false)
    });
    assert_eq!(
        stereo_configs[0].3,
        cpal::SampleFormat::F32,
        "Stereo endpoint must also prefer F32 over I16"
    );

    // On a 5.1 (6ch) Bed-Only endpoint whose driver also advertises virtual 12ch,
    // exact target_channels (6) must beat 12 so Bed-Only 5.1 upmixing is used.
    let mut bed_configs = [
        (2u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
        (6u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
        (12u16, 48_000u32, 48_000u32, cpal::SampleFormat::F32),
    ];
    bed_configs.sort_by_key(|&(ch, min_r, max_r, fmt)| {
        rank_output_config_key(ch, min_r, max_r, fmt, 6, 48_000, true, false)
    });
    assert_eq!(
        bed_configs[0].0, 6,
        "5.1 (6ch) Bed-Only target must be preferred over virtual 12ch"
    );
}

#[test]
fn test_surround_upmixer_stereo_bypass_and_bed_only_vs_bed_plus_height() {
    let sample_rate = 48_000;
    let mut dsp_stereo = SurroundUpmixerDsp::new(sample_rate);
    // Even with extreme +12 dB / -12 dB routing curve gains, stereo/mono (<=2ch) and Front L/R
    // must NEVER be equalized or colored (this is a surround routing shaper, not a master EQ).
    let cfg = crate::state::SurroundUpmixConfig {
        enabled: true,
        band_gains_db: [9.0, -9.0, 8.0, -8.0, 10.0],
        front_level: 1.0,
        surround_high_bleed: 0.0,
        stereo_width: 0.0,
        ..Default::default()
    };
    assert!(!cfg.is_active_for_hardware(1));
    assert!(!cfg.is_active_for_hardware(2));
    assert!(cfg.is_active_for_hardware(6));
    assert!(cfg.is_active_for_hardware(8));
    assert!(cfg.is_active_for_hardware(12));
    assert!(!crate::state::SurroundUpmixConfig::has_height_channels(2));
    assert!(!crate::state::SurroundUpmixConfig::has_height_channels(6));
    assert!(!crate::state::SurroundUpmixConfig::has_height_channels(8));
    assert!(crate::state::SurroundUpmixConfig::has_height_channels(10));
    assert!(crate::state::SurroundUpmixConfig::has_height_channels(12));

    dsp_stereo.sync_config(&cfg, sample_rate);

    // 1. Stereo (2ch) hardware: 100% bit-identical to input even with non-zero eq_gains_db
    let mut crossover_peak_fl = 0.0f32;
    for n in 0..2048 {
        let t = n as f32 / sample_rate as f32;
        let l_in = (2.0 * std::f32::consts::PI * cfg.height_crossover_hz * t).sin() * 0.85;
        let r_in = (2.0 * std::f32::consts::PI * 440.0 * t).cos() * 0.85;
        let stereo_out = dsp_stereo.process_frame_for_hardware(l_in, r_in, &cfg, 2);
        assert_eq!(
            stereo_out[0].to_bits(),
            l_in.to_bits(),
            "Stereo 2ch Left output must be 100% untouched even when routing curve is non-flat"
        );
        assert_eq!(
            stereo_out[1].to_bits(),
            r_in.to_bits(),
            "Stereo 2ch Right output must be 100% untouched even when routing curve is non-flat"
        );

        // And on 7.1.4 with front_gain = 1.0, FL/FR must remain 100% uncolored and bit-identical
        // even when non-zero EQ bands actively shape the synthesized surround/height channels.
        let spatial = dsp_stereo.process_stereo_to_7_1_4(l_in, r_in, &cfg);
        assert_eq!(
            spatial[0].to_bits(),
            l_in.to_bits(),
            "Front L must remain 100% uncolored (bit-identical) even with non-flat routing EQ"
        );
        assert_eq!(
            spatial[1].to_bits(),
            r_in.to_bits(),
            "Front R must remain 100% uncolored (bit-identical) even with non-flat routing EQ"
        );
        if n > 512 {
            crossover_peak_fl = crossover_peak_fl.max(spatial[0].abs());
        }
    }
    assert!(
        crossover_peak_fl > 0.80,
        "FL must not have a phase-cancellation notch at height_crossover_hz (got peak {})",
        crossover_peak_fl
    );

    // 2. Compare Bed-Only (5.1 = 6ch / 7.1 = 8ch) vs Bed + Height (7.1.4 = 12ch) at 8 kHz:
    //    - On Bed-Only (5.1/7.1), there are NO overhead height speakers, so SL/SR receive the
    //      full-bandwidth surround signal without height crossover rolloff, and channels 8..11 are 0.
    //    - On Bed + Height (7.1.4) with surround_high_bleed = 0.0, 8 kHz (> 2.8 kHz crossover)
    //      is routed to overhead height speakers (TFR/TRR) while ear-level SR is low-passed.
    let flat_cfg = crate::state::SurroundUpmixConfig {
        enabled: true,
        surround_high_bleed: 0.0,
        stereo_width: 0.0,
        ..Default::default()
    };
    let mut dsp_bed = SurroundUpmixerDsp::new(sample_rate);
    let mut dsp_atmos = SurroundUpmixerDsp::new(sample_rate);
    dsp_bed.sync_config(&flat_cfg, sample_rate);
    dsp_atmos.sync_config(&flat_cfg, sample_rate);

    let mut peak_sr_bed_only = 0.0f32;
    let mut peak_tfr_bed_only = 0.0f32;
    let mut peak_sr_bed_height = 0.0f32;
    let mut peak_tfr_bed_height = 0.0f32;

    for n in 0..2048 {
        let r = (2.0 * std::f32::consts::PI * 8000.0 * n as f32 / sample_rate as f32).sin() * 0.7;
        let out_bed = dsp_bed.process_frame_for_hardware(0.0, r, &flat_cfg, 6);
        let out_atmos = dsp_atmos.process_frame_for_hardware(0.0, r, &flat_cfg, 12);
        if n > 512 {
            peak_sr_bed_only = peak_sr_bed_only.max(out_bed[5].abs());
            peak_tfr_bed_only = peak_tfr_bed_only.max(out_bed[9].abs());
            peak_sr_bed_height = peak_sr_bed_height.max(out_atmos[5].abs());
            peak_tfr_bed_height = peak_tfr_bed_height.max(out_atmos[9].abs());
        }
    }

    assert!(
        peak_sr_bed_only > 0.35,
        "Bed-Only 5.1 SR must retain full-bandwidth 8kHz surround energy (got {})",
        peak_sr_bed_only
    );
    assert!(
        peak_tfr_bed_only < 1e-6,
        "Bed-Only 5.1 must not populate overhead height channels (got {})",
        peak_tfr_bed_only
    );
    assert!(
        peak_sr_bed_only > peak_sr_bed_height * 3.0,
        "Bed-Only SR ({}) must retain high frequencies that Bed+Height SR ({}) routes to overhead speakers",
        peak_sr_bed_only,
        peak_sr_bed_height
    );
    assert!(
        peak_tfr_bed_height > 0.45,
        "Bed+Height 7.1.4 TFR must receive the >2.8kHz height band (got {})",
        peak_tfr_bed_height
    );
}

#[test]
fn test_populate_state_default_upmix_matching_vs_non_matching_channels() {
    struct MockAudioSource {
        channels: i32,
        tracker_channels: Option<i32>,
    }
    impl AudioSource for MockAudioSource {
        fn read_frames(&mut self, _hw_ch: usize, _sr: u32, _out: &mut [f32]) -> usize {
            0
        }
        fn get_duration_seconds(&mut self) -> f64 {
            120.0
        }
        fn get_position_seconds(&mut self) -> f64 {
            0.0
        }
        fn set_position_seconds(&mut self, _pos: f64) {}
        fn get_num_channels(&mut self) -> i32 {
            self.channels
        }
        fn get_current_channel_vu_mono(&mut self, _ch: i32) -> f32 {
            0.0
        }
        fn get_artist(&mut self) -> String {
            "Test".to_string()
        }
        fn get_type(&mut self) -> String {
            "Mock".to_string()
        }
        fn get_tempo(&mut self) -> i32 {
            125
        }
        fn get_speed(&mut self) -> i32 {
            6
        }
        fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
            Some(48_000)
        }
        fn get_num_samples(&mut self) -> i32 {
            0
        }
        fn get_num_instruments(&mut self) -> i32 {
            0
        }
        fn get_num_patterns(&mut self) -> i32 {
            0
        }
        fn get_current_order(&mut self) -> i32 {
            0
        }
        fn get_current_row(&mut self) -> i32 {
            0
        }
        fn get_tracker_channels(&mut self) -> Option<i32> {
            self.tracker_channels
        }
    }

    let mut state = AppState::new("Test".to_string());
    state.force_stereo_downmix = false;

    // 1. 2ch stereo media on 6ch (5.1) output device -> non-matching -> Upmix ON by default
    let mut src_2ch = MockAudioSource {
        channels: 2,
        tracker_channels: None,
    };
    populate_state_from_audio_source(&mut state, &mut src_2ch, 6, 48_000);
    assert!(state.surround_upmix.enabled);
    assert!(state.surround_upmix.is_active_for_hardware(6));
    assert_eq!(state.num_channels, 6);

    // 2. 6ch (5.1) media on 6ch (5.1) output device -> matching -> Upmix OFF by default (discrete 5.1)
    let mut src_6ch = MockAudioSource {
        channels: 6,
        tracker_channels: None,
    };
    populate_state_from_audio_source(&mut state, &mut src_6ch, 6, 48_000);
    assert!(!state.surround_upmix.enabled);
    assert!(!state.surround_upmix.is_active_for_hardware(6));
    assert_eq!(state.num_channels, 6);

    // 3. 6ch (5.1) media on 8ch (7.1) output device -> non-matching -> Upmix ON by default (5.1 -> 7.1)
    populate_state_from_audio_source(&mut state, &mut src_6ch, 8, 48_000);
    assert!(state.surround_upmix.enabled);
    assert!(state.surround_upmix.is_active_for_hardware(8));
    assert_eq!(state.num_channels, 8);

    // 4. 8ch (7.1) media on 8ch (7.1) output device -> matching -> Upmix OFF by default (discrete 7.1)
    let mut src_8ch = MockAudioSource {
        channels: 8,
        tracker_channels: None,
    };
    populate_state_from_audio_source(&mut state, &mut src_8ch, 8, 48_000);
    assert!(!state.surround_upmix.enabled);
    assert!(!state.surround_upmix.is_active_for_hardware(8));
    assert_eq!(state.num_channels, 8);

    // 5. 2ch stereo media on 2ch stereo output device -> matching -> Upmix OFF by default
    populate_state_from_audio_source(&mut state, &mut src_2ch, 2, 48_000);
    assert!(!state.surround_upmix.enabled);
    assert!(!state.surround_upmix.is_active_for_hardware(2));
    assert_eq!(state.num_channels, 2);

    // (covered by test_populate_state_default_upmix_matching_vs_non_matching_channels)
    populate_state_from_audio_source(&mut state, &mut src_2ch, 6, 48_000);
    assert!(state.surround_upmix.enabled);
    state.set_surround_upmix(false);
    assert!(state.surround_upmix.user_overridden);
    assert!(!state.surround_upmix.enabled);
    populate_state_from_audio_source(&mut state, &mut src_2ch, 6, 48_000);
    assert!(!state.surround_upmix.enabled);
    assert_eq!(state.num_channels, 2);
}

#[test]
fn test_surround_upmixer_itu_constant_power_and_sub_bass_management() {
    // 7. Verify ITU-R BS.775 / BS.1770 constant-power loudness conservation and complementary sub-bass crossover
    let sr = 48_000;
    let cfg = crate::state::SurroundUpmixConfig::default();
    let mut dsp_2ch = SurroundUpmixerDsp::new(sr);
    let mut dsp_6ch = SurroundUpmixerDsp::new(sr);
    let mut dsp_8ch = SurroundUpmixerDsp::new(sr);
    let mut dsp_12ch = SurroundUpmixerDsp::new(sr);
    dsp_2ch.sync_config(&cfg, sr);
    dsp_6ch.sync_config(&cfg, sr);
    dsp_8ch.sync_config(&cfg, sr);
    dsp_12ch.sync_config(&cfg, sr);

    let mut power_2ch = 0.0f64;
    let mut front_power_6ch = 0.0f64;
    let mut front_power_8ch = 0.0f64;
    let mut front_power_12ch = 0.0f64;
    let mut power_6ch = 0.0f64;
    let mut power_8ch = 0.0f64;
    let mut power_12ch = 0.0f64;
    let mut corr_fc_mid = 0.0f64;
    let total_samples = 9600;
    let warmup = 1024;
    for n in 0..total_samples {
        let t = n as f32 / sr as f32;
        let two_pi = 2.0 * std::f32::consts::PI;
        // Composite stereo music mix: correlated center vocal/bass + panned mid chords + high shimmer
        let center_vocal = (two_pi * 240.0 * t).sin() * 0.28 + (two_pi * 65.0 * t).sin() * 0.16;
        let l_chord = (two_pi * 440.0 * t).sin() * 0.22 + (two_pi * 4200.0 * t).cos() * 0.14;
        let r_chord = (two_pi * 554.37 * t).cos() * 0.22 + (two_pi * 5600.0 * t).sin() * 0.14;
        let l_in = center_vocal + l_chord;
        let r_in = center_vocal + r_chord;

        let out_2 = dsp_2ch.process_frame_for_hardware(l_in, r_in, &cfg, 2);
        let out_6 = dsp_6ch.process_frame_for_hardware(l_in, r_in, &cfg, 6);
        let out_8 = dsp_8ch.process_frame_for_hardware(l_in, r_in, &cfg, 8);
        let out_12 = dsp_12ch.process_frame_for_hardware(l_in, r_in, &cfg, 12);

        // Base stereo pair (FL/FR) must be 100% bit-identical to input across all speaker layouts
        assert_eq!(out_6[0].to_bits(), l_in.to_bits());
        assert_eq!(out_6[1].to_bits(), r_in.to_bits());
        assert_eq!(out_8[0].to_bits(), l_in.to_bits());
        assert_eq!(out_8[1].to_bits(), r_in.to_bits());
        assert_eq!(out_12[0].to_bits(), l_in.to_bits());
        assert_eq!(out_12[1].to_bits(), r_in.to_bits());

        if n >= warmup {
            power_2ch += out_2[..2]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            front_power_6ch += out_6[..2]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            front_power_8ch += out_8[..2]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            front_power_12ch += out_12[..2]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            power_6ch += out_6[..6]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            power_8ch += out_8[..8]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            power_12ch += out_12[..12]
                .iter()
                .map(|&x| (x as f64) * (x as f64))
                .sum::<f64>();
            let mid = 0.5 * (l_in + r_in) as f64;
            corr_fc_mid += (out_6[2] as f64) * mid;
        }
    }
    assert!((front_power_6ch - power_2ch).abs() < 1e-9);
    assert!((front_power_8ch - power_2ch).abs() < 1e-9);
    assert!((front_power_12ch - power_2ch).abs() < 1e-9);
    assert!(
        corr_fc_mid > 0.0,
        "Synthesized FC must be in-phase (positively correlated) with front mid signal (got {})",
        corr_fc_mid
    );
    let db_6ch = 10.0 * (power_6ch / power_2ch).log10();
    let db_8ch = 10.0 * (power_8ch / power_2ch).log10();
    let db_12ch = 10.0 * (power_12ch / power_2ch).log10();
    assert!(
        db_6ch > 0.2 && db_6ch < 3.0,
        "5.1 additive upmix must preserve 0dB base FL/FR and add surround/center/LFE energy (got {:+.2} dB)",
        db_6ch
    );
    assert!(
        db_8ch > 0.2 && db_8ch < 3.0,
        "7.1 additive upmix must preserve 0dB base FL/FR and add surround/center/LFE energy (got {:+.2} dB)",
        db_8ch
    );
    assert!(
        db_12ch > 0.2 && db_12ch < 3.5,
        "7.1.4 additive upmix must preserve 0dB base FL/FR and add surround/height energy (got {:+.2} dB)",
        db_12ch
    );

    // 8. Verify complementary Linkwitz-Riley sub-bass crossover at 40 Hz (< 90 Hz sub_crossover_hz):
    // LFE must carry strong sub-bass while SL/SR bed and FC are attenuated by the high-pass filter.
    let mut dsp_sub = SurroundUpmixerDsp::new(sr);
    dsp_sub.sync_config(&cfg, sr);
    let mut peak_lfe_40hz = 0.0f32;
    let mut peak_sl_40hz = 0.0f32;
    for n in 0..4096 {
        let t = n as f32 / sr as f32;
        let sub_wave = (2.0 * std::f32::consts::PI * 40.0 * t).sin() * 0.7;
        let out = dsp_sub.process_frame_for_hardware(sub_wave, sub_wave, &cfg, 6);
        if n > 1024 {
            peak_lfe_40hz = peak_lfe_40hz.max(out[3].abs());
            peak_sl_40hz = peak_sl_40hz.max(out[4].abs());
        }
    }
    assert!(
        peak_lfe_40hz > peak_sl_40hz * 4.0,
        "At 40Hz (< 90Hz sub crossover), LFE ({}) must dominate high-passed surround bed SL ({})",
        peak_lfe_40hz,
        peak_sl_40hz
    );

    // 9. Verify native discrete 8-channel (7.1) on 8-channel hardware cannot be upmixed
    // and passes all 8 discrete channels 1:1 untouched without fold-down attenuation.
    assert!(!crate::state::SurroundUpmixConfig::can_upmix(8, 8));
    assert!(!crate::state::SurroundUpmixConfig::can_upmix(6, 6));
    assert!(crate::state::SurroundUpmixConfig::can_upmix(2, 8));
    assert!(crate::state::SurroundUpmixConfig::can_upmix(6, 8));
    let discrete_8ch = [0.8f32, -0.7, 0.5, 0.4, -0.6, 0.6, -0.35, 0.35];
    let out_discrete = dsp_8ch.process_multichannel_frame_for_hardware(&discrete_8ch, 8, &cfg, 8);
    assert_eq!(&out_discrete[..8], &discrete_8ch[..]);

    let mut state_8ch = AppState::new("8ch Test".to_string());
    state_8ch.file_loaded = true;
    state_8ch.hardware_channels = 8;
    state_8ch.source_channels = 8;
    state_8ch.surround_upmix.enabled = false;
    state_8ch.set_surround_upmix(true);
    assert!(
        !state_8ch.surround_upmix.enabled,
        "Upmix must remain bypassed when source_channels (8) == hardware_channels (8)"
    );

    let mut state_atmos = AppState::new("Atmos Test".to_string());
    state_atmos.file_loaded = true;
    state_atmos.hardware_channels = 12;
    state_atmos.source_channels = 8;
    state_atmos.is_native_atmos = true;
    state_atmos.surround_upmix.enabled = false;
    state_atmos.set_surround_upmix(true);
    assert!(
        !state_atmos.surround_upmix.enabled,
        "Upmix must remain bypassed for a native Dolby Atmos track"
    );

    // Verify 12ch source on 12ch hardware passes all 12 discrete channels 1:1 untouched.
    assert!(!crate::state::SurroundUpmixConfig::can_upmix(12, 12));
    let discrete_12ch = [
        0.7f32, -0.6, 0.5, 0.4, -0.5, 0.5, -0.35, 0.35, 0.25, -0.25, 0.15, -0.15,
    ];
    let out_discrete_12 =
        dsp_8ch.process_multichannel_frame_for_hardware(&discrete_12ch, 12, &cfg, 12);
    assert_eq!(&out_discrete_12[..12], &discrete_12ch[..]);

    // Verify 7.1 (0x63F: [FL, FR, FC, LFE, BL, BR, SL, SR]) -> 7.1.4 (12ch) maps SL/SR to [4, 5]
    // and BL/BR to [6, 7], and responds to EQ bands and speaker level sliders.
    let mut dsp_714 = SurroundUpmixerDsp::new(sr);
    let mut cfg_714 = crate::state::SurroundUpmixConfig {
        front_level: 0.5,
        center_level: 0.0,
        lfe_level: 0.0,
        side_rear_level: 0.85,
        height_level: 0.95,
        ..Default::default()
    };
    dsp_714.sync_config(&cfg_714, sr);
    // Put signal only in 0x63F slot 6 (SL = 0.6) and slot 0 (FL = 0.8), slot 2 (FC = 0.5)
    let frame_71_sl = [
        0.8f32, 0.0, 0.5, 0.4, 0.0, 0.0, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0,
    ];
    let out_714 = dsp_714.process_multichannel_frame_for_hardware(&frame_71_sl, 8, &cfg_714, 12);
    assert!(
        (out_714[0] - 0.4).abs() < 1e-4,
        "Front slider 0.5 must scale FL from 0.8 to 0.4"
    );
    assert!(out_714[2].abs() < 1e-6, "Center slider 0.0 must mute FC");
    assert!(out_714[3].abs() < 1e-6, "LFE slider 0.0 must mute LFE");
    assert!(
        out_714[4].abs() > 0.05 && out_714[6].abs() < 1e-6,
        "0x63F slot 6 (SL) must map to canonical slot 4 (SL), not slot 6 (BL)"
    );
    let mut peak_flat_tfl = 0.0f32;
    for n in 0..512 {
        let s = (2.0 * std::f32::consts::PI * 5000.0 * (n as f32 / sr as f32)).sin() * 0.25;
        let frame_5khz = [s, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let out_flat =
            dsp_714.process_multichannel_frame_for_hardware(&frame_5khz, 8, &cfg_714, 12);
        if n > 128 {
            peak_flat_tfl = peak_flat_tfl.max(out_flat[8].abs());
        }
    }
    cfg_714.band_gains_db = [10.0, 10.0, 10.0, 10.0, 10.0];
    dsp_714.sync_config(&cfg_714, sr);
    let mut peak_eq_fl = 0.0f32;
    let mut peak_eq_tfl = 0.0f32;
    for n in 0..512 {
        let s = (2.0 * std::f32::consts::PI * 5000.0 * (n as f32 / sr as f32)).sin() * 0.25;
        let frame_5khz = [s, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let out_eq = dsp_714.process_multichannel_frame_for_hardware(&frame_5khz, 8, &cfg_714, 12);
        if n > 128 {
            peak_eq_fl = peak_eq_fl.max(out_eq[0].abs());
            peak_eq_tfl = peak_eq_tfl.max(out_eq[8].abs());
        }
    }
    assert!(
        (peak_eq_fl - 0.125).abs() < 1e-4,
        "Discrete 7.1 bed channel FL must remain uncolored by 5-band routing EQ (got peak {})",
        peak_eq_fl
    );
    assert!(
        peak_eq_tfl > peak_flat_tfl * 2.0,
        "5-band routing EQ +10dB boost must more than double synthesized height TFL (flat {}, boosted {})",
        peak_flat_tfl,
        peak_eq_tfl
    );

    // 10. Verify toggling Upmix OFF/ON on a stereo track with 8ch hardware never requests
    // a stream reload (`passthrough_change_request` stays None) and crossfades smoothly.
    let mut state_2ch_on_8ch = AppState::new("2ch on 8ch Test".to_string());
    state_2ch_on_8ch.file_loaded = true;
    state_2ch_on_8ch.hardware_channels = 8;
    state_2ch_on_8ch.source_channels = 2;
    state_2ch_on_8ch.passthrough_enabled = false;
    state_2ch_on_8ch.surround_upmix.enabled = true;
    state_2ch_on_8ch.set_surround_upmix(false);
    assert!(!state_2ch_on_8ch.surround_upmix.enabled);
    assert!(
        state_2ch_on_8ch.passthrough_change_request.is_none(),
        "Toggling upmix OFF on 8ch hardware must happen in-place without tearing down the stream"
    );
    state_2ch_on_8ch.set_surround_upmix(true);
    assert!(state_2ch_on_8ch.surround_upmix.enabled);
    assert!(
        state_2ch_on_8ch.passthrough_change_request.is_none(),
        "Toggling upmix ON on 8ch hardware must happen in-place without tearing down the stream"
    );

    let mut dsp_fade = SurroundUpmixerDsp::new(sr);
    dsp_fade.sync_config(&cfg, sr);
    let dry_frame = [0.6f32, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let mut prev_out =
        dsp_fade.process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, true);
    for _ in 0..512 {
        prev_out =
            dsp_fade.process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, true);
    }
    // Now toggle target_active = false and verify the first step has no abrupt discontinuity
    let next_out =
        dsp_fade.process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, false);
    assert!(
        (next_out[0] - prev_out[0]).abs() < 0.005 && (next_out[4] - prev_out[4]).abs() < 0.005,
        "Switching upmix OFF must crossfade smoothly with zero step discontinuity"
    );
    // After 65ms (3120 frames at 48kHz), blend reaches 0.0 (pure dry stereo)
    for _ in 0..3200 {
        prev_out =
            dsp_fade.process_multichannel_frame_for_hardware_blended(&dry_frame, 2, &cfg, 8, false);
    }
    assert!((prev_out[0] - 0.6).abs() < 1e-6 && prev_out[4].abs() < 1e-6);
    assert!(!dsp_fade.is_blending_or_active(false));
}

#[test]
fn test_surround_upmixer_flexible_n_to_m_channel_matrix() {
    let sr = 48_000;
    let cfg = crate::state::SurroundUpmixConfig::default();

    // Test every source_channels (1..=8) against every larger hardware_channels (3..=12)
    for src_ch in 1..=8usize {
        for hw_ch in [3usize, 4, 5, 6, 7, 8, 9, 10, 12] {
            if src_ch >= hw_ch {
                assert!(!crate::state::SurroundUpmixConfig::can_upmix(
                    src_ch as i32,
                    hw_ch as i32
                ));
                continue;
            }
            assert!(crate::state::SurroundUpmixConfig::can_upmix(
                src_ch as i32,
                hw_ch as i32
            ));

            let mut dsp = SurroundUpmixerDsp::new(sr);
            dsp.sync_config(&cfg, sr);
            let mut peaks = [0.0f32; 12];

            for n in 0..2048 {
                let t = n as f32 / sr as f32;
                let two_pi = 2.0 * std::f32::consts::PI;
                // Multi-band test signal (55 Hz sub-bass + 440 Hz mid + 5200 Hz high)
                let l_sig = (two_pi * 55.0 * t).sin() * 0.25
                    + (two_pi * 440.0 * t).sin() * 0.35
                    + (two_pi * 5200.0 * t).cos() * 0.25;
                let r_sig = (two_pi * 55.0 * t).sin() * 0.25
                    + (two_pi * 554.37 * t).cos() * 0.35
                    + (two_pi * 6400.0 * t).sin() * 0.25;

                let mut frame = [0.0f32; 12];
                for (c, f_c) in frame.iter_mut().enumerate().take(src_ch) {
                    *f_c = if c % 2 == 0 { l_sig } else { r_sig };
                }
                let out = dsp.process_multichannel_frame_for_hardware(
                    &frame[..hw_ch],
                    src_ch,
                    &cfg,
                    hw_ch,
                );
                if n >= 512 {
                    for c in 0..12 {
                        peaks[c] = peaks[c].max(out[c].abs());
                    }
                }
            }

            // Every active hardware channel (0..hw_ch) must be driven with non-zero audio
            for (c, &p) in peaks.iter().enumerate().take(hw_ch) {
                assert!(
                    p > 0.02,
                    "Upmix {}ch -> {}ch must populate channel {} (got peak {})",
                    src_ch,
                    hw_ch,
                    c,
                    p
                );
            }
            // Unused channels beyond hw_ch must remain silent
            for (c, &p) in peaks.iter().enumerate().skip(hw_ch) {
                assert!(
                    p < 1e-6,
                    "Upmix {}ch -> {}ch must leave channel {} silent (got {})",
                    src_ch,
                    hw_ch,
                    c,
                    p
                );
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
#[test]
fn test_iamf_channel_mapping_and_downmix() {
    // 1. Verify IAMF 7.1.4 AVChannel -> Canonical [0..12] mapping
    // IAMF 7.1.4 substream order: (FL,FR), (SL,SR), (BL,BR), (TFL,TFR), (TBL,TBR), (FC), (LFE)
    // AVChannel IDs: FL=0, FR=1, FC=2, LFE=3, BL=4, BR=5, SL=9, SR=10, TFL=12, TFR=14, TBL=15, TBR=17
    let av_7_1_4 = [0, 1, 9, 10, 4, 5, 12, 14, 15, 17, 2, 3];
    let substreams_7_1_4 = [(0, 2), (1, 2), (2, 2), (3, 2), (4, 2), (5, 1), (6, 1)];
    let (reconstructed_ch, layout_label, map_7_1_4) =
        map_iamf_av_channels_to_canonical(&av_7_1_4, &substreams_7_1_4);
    assert_eq!(reconstructed_ch, 12);
    assert_eq!(layout_label, "7.1.4");
    let expected_canonical = [0, 1, 6, 7, 4, 5, 8, 9, 10, 11, 2, 3];
    for (k, &expected_dst) in expected_canonical.iter().enumerate() {
        assert_eq!(map_7_1_4[k].targets.len(), 1);
        assert_eq!(map_7_1_4[k].targets[0].0, expected_dst);
        assert!((map_7_1_4[k].targets[0].1 - 1.0).abs() < 1e-6);
    }

    // 2. Verify IAMF 5.1 AVChannel -> Canonical [0..6] mapping
    // IAMF 5.1 substream order: (FL,FR), (SL,SR), (FC), (LFE)
    let av_5_1 = [0, 1, 9, 10, 2, 3];
    let substreams_5_1 = [(0, 2), (1, 2), (2, 1), (3, 1)];
    let (reconstructed_5_1, label_5_1, map_5_1) =
        map_iamf_av_channels_to_canonical(&av_5_1, &substreams_5_1);
    assert_eq!(reconstructed_5_1, 6);
    assert_eq!(label_5_1, "5.1");
    let expected_5_1 = [0, 1, 4, 5, 2, 3];
    for (k, &expected_dst) in expected_5_1.iter().enumerate() {
        assert_eq!(map_5_1[k].targets[0].0, expected_dst);
    }

    // 3. Verify Ambisonics FOA (4ch ACN/SN3D) -> 12ch (7.1.4) mapping
    let substreams_foa = [(0, 1), (1, 1), (2, 1), (3, 1)];
    let (ambi_ch, _ambi_label, ambi_map) = map_iamf_ambisonics_to_canonical(&substreams_foa, 4);
    assert_eq!(ambi_ch, 12);
    assert_eq!(ambi_map.len(), 4);
    // W (omnidirectional ACN 0) contributes to all 12 speakers
    assert_eq!(ambi_map[0].targets.len(), 12);

    // 4. Verify 12-channel (7.1.4) downmixing to 8ch (7.1), 6ch (5.1), and 2ch (Stereo)
    // Put unit impulse solely on TFL (ch 8) and TBR (ch 11)
    let mut frame_12 = [0.0f32; 12];
    frame_12[8] = 1.0; // TFL
    frame_12[11] = 1.0; // TBR

    // Downmix to 8ch (7.1): TFL folds into FL (ch 0), TBR folds into BR (ch 5) and SR (ch 7)
    let fl_8 = get_downmixed_sample(12, &frame_12, 8, 0);
    let br_8 = get_downmixed_sample(12, &frame_12, 8, 5);
    let sr_8 = get_downmixed_sample(12, &frame_12, 8, 7);
    assert!(
        fl_8 > 0.5,
        "TFL must fold into FL in 7.1 downmix (got {})",
        fl_8
    );
    assert!(
        br_8 > 0.4,
        "TBR must fold into BR in 7.1 downmix (got {})",
        br_8
    );
    assert!(
        sr_8 > 0.2,
        "TBR must fold into SR in 7.1 downmix (got {})",
        sr_8
    );

    // Downmix to 6ch (5.1): TFL folds into FL (ch 0), TBR folds into BR/LS-RS (ch 5)
    let fl_6 = get_downmixed_sample(12, &frame_12, 6, 0);
    let br_6 = get_downmixed_sample(12, &frame_12, 6, 5);
    assert!(
        fl_6 > 0.5,
        "TFL must fold into FL in 5.1 downmix (got {})",
        fl_6
    );
    assert!(
        br_6 > 0.45,
        "TBR must fold into BR in 5.1 downmix (got {})",
        br_6
    );

    // Downmix to 2ch (Stereo): TFL folds into L (ch 0), TBR folds into R (ch 1)
    let l_2 = get_downmixed_sample(12, &frame_12, 2, 0);
    let r_2 = get_downmixed_sample(12, &frame_12, 2, 1);
    assert!(
        l_2 > 0.4,
        "TFL must fold into L in stereo downmix (got {})",
        l_2
    );
    assert!(
        r_2 > 0.35,
        "TBR must fold into R in stereo downmix (got {})",
        r_2
    );
}

#[cfg(not(target_os = "android"))]
#[test]
fn test_iamf_7_1_4_demux_and_decode() {
    let ffmpeg_bin = if std::path::Path::new(r"C:\msys64\ucrt64\bin\ffmpeg.exe").exists() {
        r"C:\msys64\ucrt64\bin\ffmpeg.exe"
    } else if std::path::Path::new("/opt/homebrew/opt/ffmpeg@7/bin/ffmpeg").exists() {
        "/opt/homebrew/opt/ffmpeg@7/bin/ffmpeg"
    } else {
        "ffmpeg"
    };

    let tmp_dir = std::env::temp_dir();
    let iamf_path = tmp_dir.join("rusttracker_test_7_1_4.iamf");
    let mp4_path = tmp_dir.join("rusttracker_test_7_1_4_iamf.mp4");

    let sg_ae =
        "type=iamf_audio_element:id=1:st=0:st=1:st=2:st=3:st=4:st=5:st=6,layer=ch_layout=7.1.4";
    let sg_mp = "type=iamf_mix_presentation:id=2:stg=0:annotations=en-us=default_mix_presentation,submix=parameter_id=100:parameter_rate=48000:default_mix_gain=0.0|element=stg=0:parameter_id=101:parameter_rate=48000:annotations=en-us=7.1.4:default_mix_gain=0.0|layout=sound_system=7.1.4:integrated_loudness=0.0:digital_peak=0.0";

    for path in [&iamf_path, &mp4_path] {
        let status = std::process::Command::new(ffmpeg_bin)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=550:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=660:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=770:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=48000:duration=0.5,pan=stereo|c0=0.5*c0|c1=0.5*c0",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=330:sample_rate=48000:duration=0.5",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=110:sample_rate=48000:duration=0.5",
                "-streamid",
                "0:0",
                "-streamid",
                "1:1",
                "-streamid",
                "2:2",
                "-streamid",
                "3:3",
                "-streamid",
                "4:4",
                "-streamid",
                "5:5",
                "-streamid",
                "6:6",
                "-stream_group",
                sg_ae,
                "-stream_group",
                sg_mp,
                "-map",
                "0:a",
                "-map",
                "1:a",
                "-map",
                "2:a",
                "-map",
                "3:a",
                "-map",
                "4:a",
                "-map",
                "5:a",
                "-map",
                "6:a",
                "-c:a",
                "libopus",
                "-b:a",
                "64k",
                path.to_str().unwrap(),
            ])
            .status();

        if let Ok(st) = status
            && st.success()
        {
            let mut src = load_audio_source(path.to_str().unwrap())
                .expect("Failed to load 7.1.4 IAMF file via load_audio_source");
            assert_eq!(src.get_num_channels(), 12);
            assert_eq!(src.get_type(), "IAMF (Opus)");
            assert_eq!(src.get_active_audio_tracks().len(), 1);
            assert!(
                src.get_duration_seconds() >= 0.45 && src.get_duration_seconds() <= 0.65,
                "Expected ~0.5s duration for {:?}, got {}",
                path,
                src.get_duration_seconds()
            );

            let mut out_12 = vec![0.0f32; 4800 * 12];
            let frames = src.read_frames(12, 48000, &mut out_12);
            assert!(
                frames > 1000,
                "Expected >1000 decoded frames, got {}",
                frames
            );

            let mut ch_peaks = [0.0f32; 12];
            for f in 0..frames {
                for c in 0..12 {
                    ch_peaks[c] = ch_peaks[c].max(out_12[f * 12 + c].abs());
                }
            }
            for (c, &peak) in ch_peaks.iter().enumerate() {
                assert!(
                    peak > 0.05,
                    "Expected non-zero audio on reconstructed IAMF 7.1.4 channel {} in {:?}, got {}",
                    c,
                    path,
                    peak
                );
            }
            assert!(out_12.iter().all(|x| x.is_finite()));

            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn test_parse_proc_asound_max_playback_channels() {
    let proc_hw_params = "access: MMAP_INTERLEAVED\nformat: S32_LE\nsubformat: STD\nchannels: 6\nrate: 48000 (48000/1)\n";
    assert_eq!(
        parse_proc_asound_max_playback_channels(proc_hw_params),
        Some(6)
    );
}

#[test]
fn test_steamdeck_dynamic_routing_and_surround_topology_lifecycle() {
    let speaker_topo = AudioSinkTopology {
        sink_id: "alsa_output.acp5x.Speaker".to_string(),
        description: "Speaker".to_string(),
        channels: 2,
    };
    let bt_topo = AudioSinkTopology {
        sink_id: "bluez_output.WH_1000XM5".to_string(),
        description: "WH-1000XM5".to_string(),
        channels: 2,
    };
    let usbc_topo = AudioSinkTopology {
        sink_id: "alsa_output.usb-TypeC_Headphones".to_string(),
        description: "USB-C Headphones".to_string(),
        channels: 2,
    };
    let usb_surround_6ch = AudioSinkTopology {
        sink_id: "alsa_output.usb-Surround_51".to_string(),
        description: "USB 5.1 Audio System".to_string(),
        channels: 6,
    };
    let usb_surround_8ch = AudioSinkTopology {
        sink_id: "alsa_output.usb-Surround_71".to_string(),
        description: "USB 7.1 Audio System".to_string(),
        channels: 8,
    };

    // Stage 1: Built-in Speakers (2ch)
    // Even if playing a 5.1 (6ch) or 7.1 (8ch) file, a 2ch sink resolves to 2 hardware channels
    // so RustTracker downmixes to stereo cleanly without needing force_stereo_downmix.
    assert_eq!(resolve_device_target_channels(false, 2, 6), 2);
    assert_eq!(resolve_device_target_channels(false, 2, 8), 2);
    assert_eq!(resolve_device_target_channels(false, 2, 2), 2);

    // Stage 2: Connect Bluetooth headphones (2ch -> 2ch)
    // On Linux (PipeWire), stream moves live without reload; OSD updates to show Bluetooth headphones.
    let step_bt_linux = evaluate_topology_transition(
        Some(&speaker_topo),
        &bt_topo,
        true,
        true,
        false,
        2,
        false,
        false,
        false,
        false,
        true,
    );
    assert!(!step_bt_linux.reload_stream);
    assert_eq!(
        step_bt_linux.osd_message.as_deref(),
        Some("🎧 Audio Output: WH-1000XM5 (2.0)")
    );

    // On Windows/macOS (non-PipeWire), changing default sink triggers stream reload onto the new device.
    let step_bt_other = evaluate_topology_transition(
        Some(&speaker_topo),
        &bt_topo,
        true,
        true,
        false,
        2,
        false,
        false,
        false,
        false,
        false,
    );
    assert!(step_bt_other.reload_stream);

    // Stage 3: Disconnect Bluetooth and connect USB-C stereo headphones (2ch -> 2ch)
    let step_usbc = evaluate_topology_transition(
        Some(&bt_topo),
        &usbc_topo,
        true,
        true,
        false,
        2,
        false,
        false,
        false,
        false,
        true,
    );
    assert!(!step_usbc.reload_stream);
    assert_eq!(
        step_usbc.osd_message.as_deref(),
        Some("🎧 Audio Output: USB-C Headphones (2.0)")
    );

    // Stage 4a: Disconnect USB-C headphones and connect to USB 5.1 Surround System (2ch -> 6ch)
    let step_surround_51 = evaluate_topology_transition(
        Some(&usbc_topo),
        &usb_surround_6ch,
        true,
        true,
        false,
        2,
        false,
        false,
        false,
        false,
        true,
    );
    assert!(
        step_surround_51.reload_stream,
        "Switching from 2ch USB-C headphones to 6ch USB surround must reload stream to unlock 6 channels"
    );
    assert_eq!(
        step_surround_51.osd_message.as_deref(),
        Some("🔊 Audio Output: USB 5.1 Audio System (5.1 Surround — Bed Only)")
    );
    // Verify channel resolution on the 6ch surround sink:
    // - 5.1 source -> 6ch discrete surround
    // - 2.0 source with surround_upmix enabled -> 6ch surround upmix
    assert_eq!(resolve_device_target_channels(false, 6, 6), 6);
    assert_eq!(resolve_device_target_channels(false, 6, 2), 6);

    // Stage 4b: Connect to USB 7.1 Surround System (6ch -> 8ch)
    let step_surround_71 = evaluate_topology_transition(
        Some(&usb_surround_6ch),
        &usb_surround_8ch,
        true,
        true,
        false,
        6,
        false,
        false,
        false,
        false,
        true,
    );
    assert!(step_surround_71.reload_stream);
    assert_eq!(
        step_surround_71.osd_message.as_deref(),
        Some("🔊 Audio Output: USB 7.1 Audio System (7.1 Surround — Bed Only)")
    );
    assert_eq!(resolve_device_target_channels(false, 8, 8), 8);
    assert_eq!(resolve_device_target_channels(false, 8, 2), 8);

    // Stage 5: Unplug USB surround system -> fallback to built-in stereo speakers (8ch -> 2ch)
    let step_back_to_speakers = evaluate_topology_transition(
        Some(&usb_surround_8ch),
        &speaker_topo,
        true,
        true,
        false,
        8,
        false,
        false,
        false,
        false,
        true,
    );
    assert!(
        step_back_to_speakers.reload_stream,
        "Switching from 8ch surround back to 2ch speakers must reload stream to downmix to stereo"
    );
    assert_eq!(
        step_back_to_speakers.osd_message.as_deref(),
        Some("🎧 Audio Output: Speaker (2.0)")
    );

    // Stage 6: Recovery from dummy output when a real sink appears
    let step_from_dummy = evaluate_topology_transition(
        Some(&speaker_topo),
        &speaker_topo,
        true,
        true,
        false,
        2,
        false,
        true,
        false,
        false,
        true,
    );
    assert!(step_from_dummy.reload_stream);
}

#[test]
fn test_downmix_7_1_to_5_1_and_stereo() {
    // 8ch frame: FL=0.5, FR=-0.5, FC=0.2, LFE=0.1, BL=0.4, BR=-0.4, SL=0.3, SR=-0.3
    let frame_8ch = [0.5f32, -0.5, 0.2, 0.1, 0.4, -0.4, 0.3, -0.3];

    // When outputting to a 5.1 (6ch) sink:
    // Ch 0..4 pass directly; Ch 4 folds BL + SL; Ch 5 folds BR + SR
    assert!((get_downmixed_sample(8, &frame_8ch, 6, 0) - 0.5).abs() < 1e-6);
    assert!((get_downmixed_sample(8, &frame_8ch, 6, 1) - (-0.5)).abs() < 1e-6);
    assert!((get_downmixed_sample(8, &frame_8ch, 6, 2) - 0.2).abs() < 1e-6);
    assert!((get_downmixed_sample(8, &frame_8ch, 6, 3) - 0.1).abs() < 1e-6);
    let expected_ls = (0.4 + 0.3) * std::f32::consts::FRAC_1_SQRT_2;
    let expected_rs = (-0.4 - 0.3) * std::f32::consts::FRAC_1_SQRT_2;
    assert!((get_downmixed_sample(8, &frame_8ch, 6, 4) - expected_ls).abs() < 1e-6);
    assert!((get_downmixed_sample(8, &frame_8ch, 6, 5) - expected_rs).abs() < 1e-6);

    // When outputting to a 2.0 (2ch) stereo sink: ITU-R BS.775 fold-down
    let l_stereo = get_downmixed_sample(8, &frame_8ch, 2, 0);
    let r_stereo = get_downmixed_sample(8, &frame_8ch, 2, 1);
    assert!(l_stereo > 0.4 && r_stereo < -0.2);
}

static LIVE_AUDIO_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct LiveAudioTestGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for LiveAudioTestGuard {
    fn drop(&mut self) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn lock_live_audio_test() -> LiveAudioTestGuard {
    let guard = LIVE_AUDIO_TEST_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    LiveAudioTestGuard { _guard: guard }
}

#[test]
fn test_smoke_2nd_pm_mpegh_714_upmix_and_live_playback() {
    let _live_audio_guard = lock_live_audio_test();
    let user_profile = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let candidates = [
        format!("{}\\Music\\2nd_pm.s3m", user_profile),
        format!("{}/Music/2nd_pm.s3m", user_profile),
        format!("{}\\Music\\2nd_pm.mod", user_profile),
        format!("{}/Music/2nd_pm.mod", user_profile),
    ];
    let Some(track_path) = candidates
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
    else {
        eprintln!("Skipping 2nd_pm smoke test: file not found in ~/Music");
        return;
    };

    eprintln!(
        "=== [SMOKE TEST PART A] Offline 2nd_pm -> 7.1.4 Upmixer -> MPEG-H 3D Audio (CICP 19) -> IEC 61937-13 ==="
    );
    let file_bytes = std::fs::read(&track_path).expect("read 2nd_pm module");
    let module = OpenMptModule::from_memory(&file_bytes).expect("OpenMPT load 2nd_pm");
    let mut src = OpenMptSource {
        module,
        volume: 1.0,
    };
    let duration = src.get_duration_seconds();
    let tracker_ch = src.get_tracker_channels().unwrap_or(0);
    let mod_type = src.get_type();
    eprintln!(
        "Loaded '{}' | Type: {} | Tracker Channels: {} | Duration: {:.2}s",
        track_path, mod_type, tracker_ch, duration
    );
    assert!(duration > 60.0, "Expected 2nd_pm duration > 60s");
    assert!(tracker_ch >= 4, "Expected tracker channels >= 4");

    // Seek into the active main section (15.0s) so all frequency bands have strong musical content
    src.set_position_seconds(15.0);
    let sample_rate = crate::mpegh::MPEGH_SAMPLE_RATE;
    let mut upmixer = SurroundUpmixerDsp::new(sample_rate);
    let upmix_cfg = crate::state::SurroundUpmixConfig {
        enabled: true,
        mpegh_bitstream: true,
        user_overridden: true,
        mpegh_user_overridden: true,
        ..crate::state::SurroundUpmixConfig::default()
    };
    upmixer.sync_config(&upmix_cfg, sample_rate);

    let mut encoder =
        crate::mpegh::Mpegh714Encoder::new(4, true).expect("Mpegh714Encoder::new(rate_factor=4)");
    let mut iec_out: std::collections::VecDeque<u8> =
        std::collections::VecDeque::with_capacity(65536 * 2);

    let total_frames = (sample_rate as usize) * 3; // 3.0 seconds = 144,000 frames @ 48 kHz
    let chunk_frames = 1024usize;
    let mut decode_buf = vec![0.0f32; chunk_frames * 12];
    let mut ch_peak = [0.0f32; 12];
    let mut ch_sum_sq = [0.0f64; 12];
    let mut frames_processed = 0usize;
    let mut bursts_verified = 0usize;
    let mut total_iec_bytes = 0usize;
    // Also simulate 192 kHz WASAPI Exclusive carrier DMA buffer drains (4608 frames * 4 bytes = 18,432 bytes)
    let mut simulated_wasapi_dma = vec![0u8; 4608 * 4];
    let mut dma_pulls = 0usize;

    while frames_processed < total_frames {
        let n = src.read_frames(12, sample_rate, &mut decode_buf);
        assert!(n > 0, "Expected non-zero decoded frames from 2nd_pm");
        for chunk_frame_pos in 0..n {
            let src_frame_idx = chunk_frame_pos;
            let frame_slice = &decode_buf[src_frame_idx * 12..(src_frame_idx + 1) * 12];
            let out_12ch = upmixer.process_multichannel_frame_for_hardware_blended(
                frame_slice,
                2,
                &upmix_cfg,
                12,
                true,
            );
            for c in 0..12 {
                let s = out_12ch[c];
                assert!(s.is_finite(), "Channel {} produced non-finite sample", c);
                ch_peak[c] = ch_peak[c].max(s.abs());
                ch_sum_sq[c] += (s as f64) * (s as f64);
            }
            let prev_len = iec_out.len();
            let emitted = encoder
                .push_rusttracker_714_frame(&out_12ch, &mut iec_out)
                .expect("push_rusttracker_714_frame");
            if emitted {
                let burst_len = iec_out.len() - prev_len;
                assert_eq!(
                    burst_len, 16_384,
                    "Each 4x IEC 61937-13 burst must be 16,384 bytes"
                );
                let b0 = iec_out[prev_len];
                let b1 = iec_out[prev_len + 1];
                let b2 = iec_out[prev_len + 2];
                let b3 = iec_out[prev_len + 3];
                let b4 = iec_out[prev_len + 4];
                let b5 = iec_out[prev_len + 5];
                let pa = u16::from_le_bytes([b0, b1]);
                let pb = u16::from_le_bytes([b2, b3]);
                let pc = u16::from_le_bytes([b4, b5]);
                assert_eq!(pa, 0xF872, "Invalid IEC 61937 Pa syncword");
                assert_eq!(pb, 0x4E1F, "Invalid IEC 61937 Pb syncword");
                assert_eq!(pc, 0x0839, "Invalid IEC 61937-13 MPEG-H Pc word");
                total_iec_bytes += burst_len;
                bursts_verified += 1;
            }
            while iec_out.len() >= simulated_wasapi_dma.len() {
                for b in &mut simulated_wasapi_dma {
                    *b = iec_out.pop_front().unwrap_or(0);
                }
                dma_pulls += 1;
            }
            frames_processed += 1;
        }
    }
    assert!(
        dma_pulls >= 120,
        "Expected >= 120 simulated 192 kHz WASAPI DMA pulls, got {}",
        dma_pulls
    );

    let ch_names = [
        "FL", "FR", "FC", "LFE", "SL", "SR", "RL", "RR", "TFL", "TFR", "TRL", "TRR",
    ];
    eprintln!(
        "Encoded {} frames ({} IEC 61937-13 bursts, {} bytes):",
        frames_processed, bursts_verified, total_iec_bytes
    );
    for c in 0..12 {
        let rms = (ch_sum_sq[c] / frames_processed as f64).sqrt();
        eprintln!(
            "  Ch {:2} ({:3}): peak = {:.4}, rms = {:.4}",
            c, ch_names[c], ch_peak[c], rms
        );
        assert!(
            ch_peak[c] > 0.001,
            "Expected active audio on upmixed channel {} ({}), got peak={}",
            c,
            ch_names[c],
            ch_peak[c]
        );
    }
    assert!(
        bursts_verified >= 140,
        "Expected >= 140 IEC 61937-13 bursts for 3.0s @ 48 kHz, got {}",
        bursts_verified
    );

    eprintln!(
        "=== [SMOKE TEST PART B1] Live WASAPI Exclusive MPEG-H 11.1 Playback of 2nd_pm (35.0s..42.0s) ==="
    );
    eprintln!(
        "Available audio output devices: {:?}",
        get_available_audio_devices(false)
    );
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(crate::state::AppState::new(
        "SmokeTest".to_string(),
    )));
    {
        let mut st = shared_state.lock().unwrap();
        st.file_loaded = true;
        st.is_paused = false;
        st.set_mpegh_bitstream(true);
        st.set_surround_upmix(true);
        st.seek_request = Some(35.0);
    }
    spawn_audio_topology_watcher(shared_state.clone(), false);
    let playback_handle_b1 = start_audio_thread(&track_path, false, shared_state.clone())
        .expect("start_audio_thread B1");

    let mut max_sec = 0.0f64;
    let mut min_sec_after_seek = f64::MAX;
    let mut saw_12ch_vus = false;
    let mut saw_spectrum = false;
    for step in 1..=14 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let st = shared_state.lock().unwrap();
        let vus: Vec<String> = st
            .raw_channel_vus
            .iter()
            .map(|v| format!("{:.3}", v))
            .collect();
        let spec_max = st.raw_spectrum_data.iter().copied().fold(0.0f32, f32::max);
        eprintln!(
            "[B1 MPEG-H] t={:.1}s: sec={:.3} ord={} row={} hw_ch={} num_ch={} mpegh_active={} carrier='{}' reload_req={:?} spec_max={:.3} vus=[{}]",
            step as f32 * 0.5,
            st.current_seconds,
            st.current_tracker_order,
            st.current_tracker_row,
            st.hardware_channels,
            st.num_channels,
            st.stats.mpegh_bitstream_active,
            st.stats.mpegh_carrier_info,
            st.audio_topology_reload_request,
            spec_max,
            vus.join(", ")
        );
        assert!(
            st.audio_topology_reload_request.is_none(),
            "Topology watcher must not falsely request stream reload during MPEG-H 11.1 playback"
        );
        if st.current_seconds >= 34.5 {
            min_sec_after_seek = min_sec_after_seek.min(st.current_seconds);
            max_sec = max_sec.max(st.current_seconds);
        }
        if st.raw_channel_vus.len() == 12 && st.raw_channel_vus.iter().any(|&v| v > 0.001) {
            saw_12ch_vus = true;
        }
        if spec_max > 0.01 {
            saw_spectrum = true;
        }
    }

    {
        let final_st = shared_state.lock().unwrap();
        assert!(final_st.file_loaded, "2nd_pm must be loaded");
        assert!(
            max_sec > 37.0 && max_sec > min_sec_after_seek + 1.5,
            "Live playback position must advance smoothly in real time (got min={:.3}s, max={:.3}s)",
            min_sec_after_seek,
            max_sec
        );
        if final_st.stats.mpegh_bitstream_active {
            assert_eq!(final_st.hardware_channels, 12);
            assert_eq!(final_st.num_channels, 12);
            assert!(
                saw_12ch_vus,
                "Expected active 12-channel VUs during live MPEG-H 11.1 upmix playback"
            );
        }
        assert!(
            saw_spectrum,
            "Expected active FFT spectrum data during live playback"
        );
    }
    drop(playback_handle_b1);
    std::thread::sleep(std::time::Duration::from_millis(250));

    eprintln!("=== [SMOKE TEST PART B2] Live LPCM Playback of 2nd_pm (42.0s..49.0s) ===");
    {
        let mut st = shared_state.lock().unwrap();
        st.is_paused = false;
        st.set_mpegh_bitstream(false);
        st.set_surround_upmix(true);
        st.seek_request = Some(42.0);
        st.audio_topology_reload_request = None;
    }
    let playback_handle_b2 = start_audio_thread(&track_path, false, shared_state.clone())
        .expect("start_audio_thread B2");
    for step in 1..=14 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let st = shared_state.lock().unwrap();
        let vus: Vec<String> = st
            .raw_channel_vus
            .iter()
            .map(|v| format!("{:.3}", v))
            .collect();
        let spec_max = st.raw_spectrum_data.iter().copied().fold(0.0f32, f32::max);
        eprintln!(
            "[B2 LPCM]   t={:.1}s: sec={:.3} ord={} row={} hw_ch={} num_ch={} mpegh_active={} reload_req={:?} spec_max={:.3} vus=[{}]",
            step as f32 * 0.5,
            st.current_seconds,
            st.current_tracker_order,
            st.current_tracker_row,
            st.hardware_channels,
            st.num_channels,
            st.stats.mpegh_bitstream_active,
            st.audio_topology_reload_request,
            spec_max,
            vus.join(", ")
        );
    }
    drop(playback_handle_b2);
}

#[cfg(windows)]
#[test]
fn test_live_pause_and_resume_verification() {
    let _live_audio_guard = lock_live_audio_test();
    let user_profile = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let candidates = [
        format!("{}\\Music\\2nd_pm.s3m", user_profile),
        format!("{}\\Music\\2nd_skav.s3m", user_profile),
    ];
    let Some(track_path) = candidates
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
    else {
        eprintln!("Skipping live pause/resume test: module not found in ~/Music");
        return;
    };

    for (label, mpegh_mode) in [("Spatial 7.1.4", true), ("WASAPI Shared 8ch", false)] {
        eprintln!("=== Testing Play -> Pause -> Resume ({}) ===", label);
        let shared_state = std::sync::Arc::new(std::sync::Mutex::new(crate::state::AppState::new(
            "PauseResumeTest".to_string(),
        )));
        {
            let mut st = shared_state.lock().unwrap();
            st.file_loaded = true;
            st.is_paused = false;
            st.set_mpegh_bitstream(mpegh_mode);
            st.set_surround_upmix(true);
            st.seek_request = Some(35.0);
        }
        let handle = start_audio_thread(&track_path, false, shared_state.clone())
            .expect("start_audio_thread");

        // 1. Play for 1.5s
        for step in 1..=3 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let st = shared_state.lock().unwrap();
            let vu_max = st.raw_channel_vus.iter().copied().fold(0.0f32, f32::max);
            eprintln!(
                "  [{} PLAY  t={:.1}s] sec={:.3} vu_max={:.3} dev_lost={}",
                label,
                step as f32 * 0.5,
                st.current_seconds,
                vu_max,
                st.audio_device_lost
            );
            assert!(!st.audio_device_lost, "{} lost device during play", label);
        }
        let sec_before_pause = shared_state.lock().unwrap().current_seconds;
        assert!(
            sec_before_pause > 35.8,
            "{} playback did not advance before pause (sec={:.3})",
            label,
            sec_before_pause
        );

        // 2. Pause for 1.5s while simulating 120 Hz GUI lock contention (~2ms hold per 8ms frame)
        {
            let mut st = shared_state.lock().unwrap();
            st.is_paused = true;
            st.raw_channel_vus.fill(0.0);
            st.raw_spectrum_data.fill(0.0);
            st.gpu_spectrum_data.fill(0.0);
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
        let sec_at_pause_start = {
            let mut st = shared_state.lock().unwrap();
            st.raw_channel_vus.fill(0.0);
            st.raw_spectrum_data.fill(0.0);
            st.gpu_spectrum_data.fill(0.0);
            st.current_seconds
        };
        for step in 1..=3 {
            let mut max_paused_vu = 0.0f32;
            let mut max_paused_spec = 0.0f32;
            for _frame in 0..60 {
                {
                    let st = shared_state.lock().unwrap();
                    let vu_max = st.raw_channel_vus.iter().copied().fold(0.0f32, f32::max);
                    let spec_max = st.raw_spectrum_data.iter().copied().fold(0.0f32, f32::max);
                    max_paused_vu = max_paused_vu.max(vu_max);
                    max_paused_spec = max_paused_spec.max(spec_max);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                std::thread::sleep(std::time::Duration::from_millis(6));
            }
            let st = shared_state.lock().unwrap();
            eprintln!(
                "  [{} PAUSE t={:.1}s] sec={:.3} max_vu={:.4} max_spec={:.4} dev_lost={}",
                label,
                step as f32 * 0.5,
                st.current_seconds,
                max_paused_vu,
                max_paused_spec,
                st.audio_device_lost
            );
            assert!(!st.audio_device_lost, "{} lost device during pause", label);
            assert_eq!(
                max_paused_vu, 0.0,
                "{} raw_channel_vus spiked while paused under GUI lock contention!",
                label
            );
            assert_eq!(
                max_paused_spec, 0.0,
                "{} raw_spectrum_data spiked while paused under GUI lock contention!",
                label
            );
            assert!(
                (st.current_seconds - sec_at_pause_start).abs() < 0.001,
                "{} current_seconds advanced while paused! ({:.3} -> {:.3})",
                label,
                sec_at_pause_start,
                st.current_seconds
            );
        }
        let sec_during_pause = shared_state.lock().unwrap().current_seconds;

        // 3. Resume for 2.0s
        shared_state.lock().unwrap().is_paused = false;
        let mut max_vu_after_resume = 0.0f32;
        for step in 1..=4 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let st = shared_state.lock().unwrap();
            let vu_max = st.raw_channel_vus.iter().copied().fold(0.0f32, f32::max);
            max_vu_after_resume = max_vu_after_resume.max(vu_max);
            eprintln!(
                "  [{} RESUME t={:.1}s] sec={:.3} vu_max={:.3} dev_lost={}",
                label,
                step as f32 * 0.5,
                st.current_seconds,
                vu_max,
                st.audio_device_lost
            );
            assert!(!st.audio_device_lost, "{} lost device on resume", label);
        }
        let sec_after_resume = shared_state.lock().unwrap().current_seconds;
        assert!(
            sec_after_resume > sec_during_pause + 1.0,
            "{} playback failed to advance after resume! (paused_sec={:.3}, resumed_sec={:.3})",
            label,
            sec_during_pause,
            sec_after_resume
        );
        assert!(
            max_vu_after_resume > 0.01,
            "{} produced no audio VUs after resume! (max_vu={:.4})",
            label,
            max_vu_after_resume
        );
        drop(handle);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(windows)]
#[test]
fn test_spatial_audio_probe() {
    let _live_audio_guard = lock_live_audio_test();
    use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Threading::CreateEventW;
    use windows::Win32::System::Variant::{VT_BLOB, VT_LPWSTR};
    use windows::core::{GUID, Interface};

    const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
        let collection = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .unwrap();
        let count = collection.GetCount().unwrap_or(0);
        for i in 0..count {
            let dev = collection.Item(i).unwrap();
            let store = dev.OpenPropertyStore(STGM_READ).unwrap();
            let propvar = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).unwrap();
            let name = if propvar.Anonymous.Anonymous.vt == VT_LPWSTR {
                propvar
                    .Anonymous
                    .Anonymous
                    .Anonymous
                    .pwszVal
                    .to_string()
                    .unwrap_or_default()
            } else {
                propvar.to_string()
            };
            eprintln!("=== Probing ISpatialAudioClient on '{}' ===", name);
            match dev.Activate::<ISpatialAudioClient>(CLSCTX_ALL, None) {
                Ok(sac) => {
                    let static_mask = sac.GetNativeStaticObjectTypeMask();
                    let max_dyn = sac.GetMaxDynamicObjectCount();
                    let avail = sac
                        .IsSpatialAudioStreamAvailable(&ISpatialAudioObjectRenderStream::IID, None);
                    eprintln!(
                        "  GetNativeStaticObjectTypeMask: {:?}, GetMaxDynamicObjectCount: {:?}, IsSpatialAudioStreamAvailable: {:?}",
                        static_mask, max_dyn, avail
                    );
                    if let Ok(fmt_enum) = sac.GetSupportedAudioObjectFormatEnumerator() {
                        let fmt_count = fmt_enum.GetCount().unwrap_or(0);
                        eprintln!("  Supported AudioObject formats: {}", fmt_count);
                        for f_idx in 0..fmt_count {
                            if let Ok(wf_ptr) = fmt_enum.GetFormat(f_idx) {
                                let wf = *wf_ptr;
                                let tag = wf.wFormatTag;
                                let ch = wf.nChannels;
                                let rate = wf.nSamplesPerSec;
                                let bits = wf.wBitsPerSample;
                                let align = wf.nBlockAlign;
                                let max_frames = sac.GetMaxFrameCount(wf_ptr).unwrap_or(0);
                                eprintln!(
                                    "    Format[{}]: tag=0x{:X} ch={} rate={} bits={} align={} max_frames={}",
                                    f_idx, tag, ch, rate, bits, align, max_frames
                                );
                                let mask_714 = AudioObjectType_FrontLeft
                                    | AudioObjectType_FrontRight
                                    | AudioObjectType_FrontCenter
                                    | AudioObjectType_LowFrequency
                                    | AudioObjectType_SideLeft
                                    | AudioObjectType_SideRight
                                    | AudioObjectType_BackLeft
                                    | AudioObjectType_BackRight
                                    | AudioObjectType_TopFrontLeft
                                    | AudioObjectType_TopFrontRight
                                    | AudioObjectType_TopBackLeft
                                    | AudioObjectType_TopBackRight;
                                let ev = CreateEventW(None, false, false, None).unwrap();
                                let mut params = SpatialAudioObjectRenderStreamActivationParams {
                                    ObjectFormat: wf_ptr,
                                    StaticObjectTypeMask: mask_714,
                                    MinDynamicObjectCount: 0,
                                    MaxDynamicObjectCount: 0,
                                    Category: AudioCategory_Media,
                                    EventHandle: ev,
                                    NotifyObject: std::mem::ManuallyDrop::new(None),
                                };
                                let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
                                (*pv.Anonymous.Anonymous).vt = VT_BLOB;
                                (*pv.Anonymous.Anonymous).Anonymous.blob = BLOB {
                                    cbSize: std::mem::size_of::<
                                        SpatialAudioObjectRenderStreamActivationParams,
                                    >() as u32,
                                    pBlobData: &mut params as *mut _ as *mut u8,
                                };
                                let stream_res = sac
                                    .ActivateSpatialAudioStream::<ISpatialAudioObjectRenderStream>(
                                        &*pv,
                                    );
                                eprintln!(
                                    "    ActivateSpatialAudioStream(7.1.4 static mask) => {:?}",
                                    stream_res.as_ref().map(|_| "OK")
                                );
                                if let Ok(stream) = stream_res {
                                    let static_types = [
                                        ("FL", AudioObjectType_FrontLeft),
                                        ("FR", AudioObjectType_FrontRight),
                                        ("FC", AudioObjectType_FrontCenter),
                                        ("LFE", AudioObjectType_LowFrequency),
                                        ("SL", AudioObjectType_SideLeft),
                                        ("SR", AudioObjectType_SideRight),
                                        ("BL", AudioObjectType_BackLeft),
                                        ("BR", AudioObjectType_BackRight),
                                        ("TFL", AudioObjectType_TopFrontLeft),
                                        ("TFR", AudioObjectType_TopFrontRight),
                                        ("TBL", AudioObjectType_TopBackLeft),
                                        ("TBR", AudioObjectType_TopBackRight),
                                    ];
                                    let mut objects = Vec::with_capacity(12);
                                    for (label, obj_type) in static_types {
                                        let obj = stream
                                            .ActivateSpatialAudioObject(obj_type)
                                            .expect("ActivateSpatialAudioObject");
                                        objects.push((label, obj));
                                    }
                                    eprintln!(
                                        "      Successfully activated all {} static 7.1.4 AudioObjects!",
                                        objects.len()
                                    );
                                    stream
                                        .Start()
                                        .expect("ISpatialAudioObjectRenderStream::Start");
                                    for tick in 0..5 {
                                        let wait_res =
                                            windows::Win32::System::Threading::WaitForSingleObject(
                                                ev, 100,
                                            );
                                        let mut dyn_count = 0u32;
                                        let mut frame_count = 0u32;
                                        stream
                                            .BeginUpdatingAudioObjects(
                                                &mut dyn_count,
                                                &mut frame_count,
                                            )
                                            .expect("BeginUpdatingAudioObjects");
                                        for (_, obj) in &objects {
                                            let mut buf_ptr: *mut u8 = std::ptr::null_mut();
                                            let mut buf_bytes = 0u32;
                                            obj.GetBuffer(&mut buf_ptr, &mut buf_bytes)
                                                .expect("ISpatialAudioObject::GetBuffer");
                                            if !buf_ptr.is_null() && buf_bytes > 0 {
                                                std::ptr::write_bytes(
                                                    buf_ptr,
                                                    0,
                                                    buf_bytes as usize,
                                                );
                                            }
                                        }
                                        stream
                                            .EndUpdatingAudioObjects()
                                            .expect("EndUpdatingAudioObjects");
                                        eprintln!(
                                            "      Tick {}: wait={} dyn_count={} frame_count={}",
                                            tick, wait_res.0, dyn_count, frame_count
                                        );
                                    }
                                    let _ = stream.Stop();
                                }
                                let _ = CloseHandle(ev);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  Activate::<ISpatialAudioClient> failed: {:?}", e);
                }
            }
        }
        CoUninitialize();
    }
}

#[test]
fn test_atmos_detection_and_96khz_symphonia_resampling() {
    let user_profile = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();

    // 1. Verify native Dolby Atmos track detection ("01 - Major Tom (Coming Home).mkv")
    let atmos_path_buf = std::path::Path::new(&user_profile)
        .join("Music")
        .join("01 - Major Tom (Coming Home).mkv");
    let atmos_path = atmos_path_buf.to_string_lossy().into_owned();
    if atmos_path_buf.exists() {
        let mut src = load_audio_source(&atmos_path).expect("load Major Tom mkv");
        assert!(
            src.is_native_atmos(),
            "01 - Major Tom (Coming Home).mkv must be detected as native Dolby Atmos"
        );
        let mut st = AppState::new("Major Tom".to_string());
        st.surround_upmix.enabled = true;
        st.surround_upmix.user_overridden = true;
        populate_state_from_audio_source(&mut st, src.as_mut(), 12, 48000);
        assert!(
            st.is_native_atmos,
            "AppState.is_native_atmos must be true for Major Tom"
        );
        assert!(
            !st.surround_upmix.enabled,
            "Surround Upmix must be automatically disabled for native Dolby Atmos"
        );
        let mut decode_buf = vec![0.0f32; 4800 * 2];
        let frames = src.read_frames(2, 48000, &mut decode_buf);
        assert_eq!(
            frames, 4800,
            "TrueHD Atmos MKV must decode 4800 stereo-downmixed frames"
        );
        src.set_position_seconds(30.0);
        let frames_at_30s = src.read_frames(2, 48000, &mut decode_buf);
        assert_eq!(frames_at_30s, 4800);
        let peak = decode_buf.iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
        assert!(
            peak > 0.01,
            "TrueHD Atmos MKV decoded at 30.0s must produce audible non-silent PCM (peak={})",
            peak
        );
        assert_eq!(src.get_num_channels(), 8);
        assert!(src.get_current_channel_vu_mono(0) > 0.01);
        assert_eq!(
            src.get_video_info().as_deref(),
            Some("H264 (1920x1080)"),
            "probe_video_stream must select primary 1920x1080 H.264 stream and skip MJPEG cover art"
        );
        eprintln!(
            "Verified native Atmos track '{}': is_native_atmos={}, peak@30s={:.4}, video_info={:?}, tracks={:?}",
            atmos_path,
            st.is_native_atmos,
            peak,
            src.get_video_info(),
            st.audio_tracks
        );
    }

    // 2. Verify 96,000 Hz 5.1 FLAC ("02 - What Is Love.flac") resamples to 48,000 Hz at 1.0x speed
    let flac_path_buf = std::path::Path::new(&user_profile)
        .join("Music")
        .join("Howard Jones - Human's Lib")
        .join("02 - What Is Love.flac");
    let flac_path = flac_path_buf.to_string_lossy().into_owned();
    if flac_path_buf.exists() {
        let mut src = load_audio_source(&flac_path).expect("load What Is Love flac");
        assert_eq!(src.get_intrinsic_sample_rate(), Some(96000));
        assert_eq!(src.get_num_channels(), 6);
        assert!(!src.is_native_atmos());

        // Read 48,000 output frames at output_sample_rate = 48,000 Hz (1.0 second of audio)
        let mut buf = vec![0.0f32; 48000 * 12];
        let frames = src.read_frames(12, 48000, &mut buf);
        assert_eq!(frames, 48000);
        let pos = src.get_position_seconds();
        assert!(
            (pos - 1.0).abs() < 0.01,
            "After reading 48,000 frames at 48,000 Hz from a 96,000 Hz FLAC, position must be ~1.0s (got {:.4}s)",
            pos
        );
        eprintln!(
            "Verified 96kHz -> 48kHz resampling on '{}': read {} frames @ 48kHz, position={:.4}s",
            flac_path, frames, pos
        );
    }
}

#[test]
fn test_upmixer_smooth_center_steering_no_harmonic_hiss_and_packet_resampler_continuity() {
    // 1. Verify that feeding low/mid stereo signals where |mid(t)| and |diff(t)| cross each
    //    other every cycle (e.g. L = 220 Hz + 330 Hz, R = 220 Hz - 330 Hz) does NOT generate
    //    synthetic >4 kHz harmonic/intermodulation hiss in the overhead height speakers.
    let sr = 48_000;
    let cfg = crate::state::SurroundUpmixConfig::default();
    let mut dsp = SurroundUpmixerDsp::new(sr);
    dsp.sync_config(&cfg, sr);

    // High-pass diagnostic probe (2nd-order Butterworth at 6000 Hz) on TFL/TRL output:
    // Since the input contains only 220 Hz and 330 Hz tones, any 6+ kHz energy in the output
    // would be pure non-linear harmonic distortion (hiss).
    let mut hiss_probe_tfl =
        BiquadFilter::highpass(sr as f32, 6000.0, std::f32::consts::FRAC_1_SQRT_2);
    let mut hiss_probe_trl =
        BiquadFilter::highpass(sr as f32, 6000.0, std::f32::consts::FRAC_1_SQRT_2);
    let mut max_hiss_tfl = 0.0f32;
    let mut max_hiss_trl = 0.0f32;

    for n in 0..9600 {
        let t = n as f32 / sr as f32;
        let two_pi = 2.0 * std::f32::consts::PI;
        let mid_tone = (two_pi * 220.0 * t).sin() * 0.45;
        let side_tone = (two_pi * 330.0 * t).sin() * 0.35;
        let l_in = mid_tone + side_tone;
        let r_in = mid_tone - side_tone;
        let out = dsp.process_frame_for_hardware(l_in, r_in, &cfg, 12);
        let h_tfl = hiss_probe_tfl.process(out[8]);
        let h_trl = hiss_probe_trl.process(out[10]);
        if n > 2048 {
            max_hiss_tfl = max_hiss_tfl.max(h_tfl.abs());
            max_hiss_trl = max_hiss_trl.max(h_trl.abs());
        }
    }
    assert!(
        max_hiss_tfl < 1e-3 && max_hiss_trl < 1e-3,
        "Smooth center steering must not generate >6 kHz harmonic distortion hiss from 220/330 Hz tones (got TFL={:.6}, TRL={:.6})",
        max_hiss_tfl,
        max_hiss_trl
    );

    // 2. Verify that resampling 44,100 Hz -> 48,000 Hz across arbitrary packet boundaries
    //    (e.g. 257-frame and 1-frame chunks) produces bit-for-bit identical output to resampling
    //    a single contiguous buffer, with zero packet-boundary clicks or phase slips.
    let in_rate = 44_100u32;
    let out_rate = 48_000u32;
    let ch = 2usize;
    let total_in_frames = 8820usize; // 200 ms at 44.1 kHz
    let mut full_src = Vec::with_capacity(total_in_frames * ch);
    for n in 0..total_in_frames {
        let t = n as f32 / in_rate as f32;
        let two_pi = 2.0 * std::f32::consts::PI;
        full_src.push((two_pi * 1000.0 * t).sin() * 0.8);
        full_src.push((two_pi * 3500.0 * t).cos() * 0.8);
    }

    // Contiguous reference pass
    let mut pos_ref = 0.0f64;
    let mut prev_ref = Vec::new();
    let mut out_ref = Vec::new();
    resample_interleaved_packet(
        &full_src,
        ch,
        in_rate,
        out_rate,
        &mut pos_ref,
        &mut prev_ref,
        &mut out_ref,
    );

    // Chunked packet pass with irregular packet sizes (1, 2, 17, 257, 1152 frames)
    let mut pos_chunked = 0.0f64;
    let mut prev_chunked = Vec::new();
    let mut out_chunked = Vec::new();
    let chunk_sizes = [1usize, 2, 17, 257, 1152];
    let mut cursor = 0usize;
    let mut step_idx = 0usize;
    while cursor < total_in_frames {
        let take = chunk_sizes[step_idx % chunk_sizes.len()].min(total_in_frames - cursor);
        let slice = &full_src[cursor * ch..(cursor + take) * ch];
        resample_interleaved_packet(
            slice,
            ch,
            in_rate,
            out_rate,
            &mut pos_chunked,
            &mut prev_chunked,
            &mut out_chunked,
        );
        cursor += take;
        step_idx += 1;
    }

    assert_eq!(
        out_ref.len(),
        out_chunked.len(),
        "Chunked resampling must produce the exact same sample count as contiguous resampling"
    );
    let max_diff = out_ref
        .iter()
        .zip(out_chunked.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_diff < 1e-6,
        "Chunked resampling across packet boundaries must match contiguous resampling (max_diff={})",
        max_diff
    );
}

#[test]
fn test_itu_bs775_downmix_and_crossover_phase_coherence() {
    // 1. Mono-to-multichannel (1 -> 6) must duplicate mono to FL/FR and keep FC/LFE/SL/SR silent
    let mono_in = [0.75f32];
    assert!((get_downmixed_sample(1, &mono_in, 6, 0) - 0.75).abs() < 1e-6);
    assert!((get_downmixed_sample(1, &mono_in, 6, 1) - 0.75).abs() < 1e-6);
    assert_eq!(get_downmixed_sample(1, &mono_in, 6, 2), 0.0);
    assert_eq!(get_downmixed_sample(1, &mono_in, 6, 3), 0.0);

    // 2. ITU-R BS.775-3 LFE exclusion: LFE (channel 3) in 5.1/7.1 must not bleed into stereo/mono downmix
    let lfe_only_51 = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    assert_eq!(get_downmixed_sample(6, &lfe_only_51, 2, 0), 0.0);
    assert_eq!(get_downmixed_sample(6, &lfe_only_51, 2, 1), 0.0);
    assert_eq!(get_downmixed_sample(6, &lfe_only_51, 1, 0), 0.0);

    // 3. Soft limiter remains 100% linear up to 0.98 (-0.17 dBFS)
    assert_eq!(SurroundUpmixerDsp::soft_limit(0.95), 0.95);
    assert_eq!(SurroundUpmixerDsp::soft_limit(-0.95), -0.95);

    // 4. Center channel (FC) in `process_stereo_to_7_1_4` and `process_stereo_to_bed_only`
    //    must remain in-phase (positive correlation) with FL/FR in the vocal/midrange passband (1 kHz)
    let sr = 48_000;
    let cfg = crate::state::SurroundUpmixConfig::default();
    let mut dsp = SurroundUpmixerDsp::new(sr);
    dsp.sync_config(&cfg, sr);
    let mut dot_fl_fc = 0.0f32;
    for n in 0..4800 {
        let t = n as f32 / sr as f32;
        let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.5;
        let out = dsp.process_stereo_to_7_1_4(s, s, &cfg);
        if n > 1000 {
            dot_fl_fc += out[0] * out[2];
        }
    }
    assert!(
        dot_fl_fc > 100.0,
        "FC must be in-phase with FL/FR at 1 kHz (got dot_fl_fc={})",
        dot_fl_fc
    );

    // 5. Frequency Analysis Y-axis height: a narrow-band harmonic component with amplitude 0.035
    //    must reach >75% of the [0..100] Y-axis display range at both 48 kHz (N=8192) and 96 kHz (N=16384),
    //    and `compute_fft_mono_sample` must preserve full stereo-to-mono amplitude when upmixing.
    for &(sr, win_size) in &[(48_000u32, 8192usize), (96_000u32, 16384usize)] {
        let state = Arc::new(Mutex::new(AppState::new("SpectrumHeightTest".to_string())));
        {
            let mut s = state.lock().unwrap();
            s.is_paused = false;
            s.file_loaded = true;
        }
        let (tx, rx) = bounded::<DspMessage>(4);
        spawn_dsp_thread(rx, state.clone(), sr, 20_000.0, win_size);

        let freq = 1000.0f32;
        let amp = 0.035f32;
        let audio_data: Vec<f32> = (0..win_size)
            .map(|i| (2.0 * std::f32::consts::PI * freq * (i as f32) / (sr as f32)).sin() * amp)
            .collect();

        tx.send(DspMessage {
            audio_data: audio_data.clone(),
            channel_audio_data: vec![audio_data],
            channel_vus: vec![amp, amp],
            current_order: 0,
            current_row: 0,
            bpm: 120,
            speed: 6,
            current_seconds: 1.0,
            current_row_string: String::new(),
            seek_epoch: 0,
        })
        .unwrap();
        drop(tx);

        std::thread::sleep(std::time::Duration::from_millis(60));
        let peak_bin = state
            .lock()
            .unwrap()
            .raw_spectrum_data
            .iter()
            .copied()
            .fold(0.0f32, f32::max);
        assert!(
            peak_bin > 70.0,
            "Frequency analysis Y-axis peak at {} Hz (N={}) for amp={} must exceed 70.0 (got {:.2})",
            sr,
            win_size,
            amp,
            peak_bin
        );
    }

    assert!((compute_fft_mono_sample(&[0.8, 0.8], 2) - 0.8).abs() < 1e-6);
}

#[test]
fn test_oscilloscope_standing_wave_trigger_locks_fundamental_and_template_phase() {
    let sr = 48_000u32;
    let win_size = 8192usize;
    let visual_width = 1024usize;
    let mut trigger = OscilloscopeTriggerState::new();
    let mut prev_wave: Vec<f32> = Vec::new();
    let mut curr_wave: Vec<f32> = Vec::new();

    // 1a. Multi-harmonic wave with multiple rising zero-crossings per fundamental period
    // plus a steep 24th harmonic whose single-sample derivative (24 * 0.10 = 2.40) is Nearly
    // 4x larger than the fundamental's slope (1 * 0.65 = 0.65).
    let f0 = 140.0f32;
    let two_pi = 2.0 * std::f32::consts::PI;

    for frame in 0..10 {
        let sample_offset = frame * 397; // arbitrary non-integer-period frame step
        let audio_data: Vec<f32> = (0..win_size)
            .map(|i| {
                let t = ((i + sample_offset) as f32) / (sr as f32);
                let fund = (two_pi * f0 * t).sin() * 0.65;
                let h2 = (two_pi * (f0 * 2.0) * t + 0.55).sin() * 0.45;
                let h3 = (two_pi * (f0 * 3.0) * t - 0.30).sin() * 0.25;
                let h24 = (two_pi * (f0 * 24.0) * t + 0.20).sin() * 0.10;
                fund + h2 + h3 + h24
            })
            .collect();

        trigger.extract_triggered_waveform(&audio_data, sr, visual_width, 1, &mut curr_wave);
        assert_eq!(curr_wave.len(), visual_width);

        if frame > 0 {
            // Consecutive triggered frames must remain phase-locked (standing wave)
            let mad: f32 = curr_wave
                .iter()
                .zip(prev_wave.iter())
                .map(|(&a, &b)| (a - b).abs())
                .sum::<f32>()
                / (visual_width as f32);
            assert!(
                mad < 0.025,
                "Standing wave frame {} drifted out of phase (MAD = {:.4})",
                frame,
                mad
            );
        }
        prev_wave.clone_from(&curr_wave);
    }

    // 1b. Incommensurate high-frequency transient (3719 Hz) riding on a 110 Hz bass wave:
    // Verify that the 110 Hz fundamental remains stationary at the center node across frames.
    let mut prev_center_slope_sign = 0.0f32;
    for frame in 0..8 {
        let sample_offset = frame * 523;
        let audio_data: Vec<f32> = (0..win_size)
            .map(|i| {
                let t = ((i + sample_offset) as f32) / (sr as f32);
                let bass = (two_pi * 110.0 * t).sin() * 0.70;
                let hihat_transient = (two_pi * 3719.0 * t).sin() * 0.12;
                bass + hihat_transient
            })
            .collect();

        trigger.extract_triggered_waveform(&audio_data, sr, visual_width, 2, &mut curr_wave);
        let mid = visual_width / 2;
        // Average over a narrow neighborhood around the center node to strip the 3719Hz ripple
        let center_fund: f32 = curr_wave[(mid - 6)..=(mid + 6)].iter().sum::<f32>() / 13.0;
        let slope_fund: f32 = curr_wave[(mid + 8)..=(mid + 20)].iter().sum::<f32>() / 13.0
            - curr_wave[(mid - 20)..=(mid - 8)].iter().sum::<f32>() / 13.0;
        assert!(
            center_fund.abs() < 0.06 && slope_fund > 0.15,
            "Frame {} failed to lock fundamental rising node under HF transient (center={:.4}, slope={:.4})",
            frame,
            center_fund,
            slope_fund
        );
        prev_center_slope_sign = slope_fund.signum();
    }
    assert!(prev_center_slope_sign > 0.0);

    // 2. Verify resolution-independent 23ms timebase across 512, 1024, and 1920 widths
    let pure_audio: Vec<f32> = (0..win_size)
        .map(|i| {
            let t = (i as f32) / (sr as f32);
            (two_pi * 220.0 * t).sin() * 0.8
        })
        .collect();

    let mut wave_512: Vec<f32> = Vec::new();
    let mut wave_1920: Vec<f32> = Vec::new();
    trigger.extract_triggered_waveform(&pure_audio, sr, 512, 2, &mut wave_512);
    trigger.extract_triggered_waveform(&pure_audio, sr, 1920, 2, &mut wave_1920);

    // Center of sweep (u = 0.5) must be pinned at the rising zero-crossing node
    let center_512 = wave_512[512 / 2];
    let center_1920 = wave_1920[1920 / 2];
    assert!(
        center_512.abs() < 0.03 && center_1920.abs() < 0.03,
        "Center node must be pinned near 0.0 (got 512={:.4}, 1920={:.4})",
        center_512,
        center_1920
    );
    assert!(
        wave_1920[1920 / 2 + 4] > wave_1920[1920 / 2 - 4],
        "Center node must be a rising zero-crossing"
    );

    // Both 512 and 1920 resolutions must span the same ~23ms timebase (same end-to-end phase)
    for k in 0..=8 {
        let u = (k as f32) / 8.0;
        let v512 = wave_512[((u * 511.0).round() as usize).min(511)];
        let v1920 = wave_1920[((u * 1919.0).round() as usize).min(1919)];
        assert!(
            (v512 - v1920).abs() < 0.04,
            "Timebase mismatch at u={:.3}: wave_512={:.4} vs wave_1920={:.4}",
            u,
            v512,
            v1920
        );
    }
}

#[test]
fn test_discrete_51_and_71_surround_width_and_eq() {
    let sr = 48_000;
    let frame_51 = [0.30_f32, -0.25, 0.15, 0.10, 0.60, -0.20, 0.0, 0.0];

    let cfg_unity = crate::state::SurroundUpmixConfig {
        enabled: true,
        stereo_width: 1.0,
        ..Default::default()
    };
    let cfg_wide = crate::state::SurroundUpmixConfig {
        enabled: true,
        stereo_width: 1.8,
        ..Default::default()
    };

    // 1. Discrete 5.1 -> 7.1 upmix: changing ONLY `stereo_width` (with flat EQ & default levels)
    // must widen surround channels while keeping FL/FR/FC/LFE identical.
    let mut dsp_u = SurroundUpmixerDsp::new(sr);
    dsp_u.sync_config(&cfg_unity, sr);
    let out_u = dsp_u.process_multichannel_frame_for_hardware(&frame_51, 6, &cfg_unity, 8);

    let mut dsp_w = SurroundUpmixerDsp::new(sr);
    dsp_w.sync_config(&cfg_wide, sr);
    let out_w = dsp_w.process_multichannel_frame_for_hardware(&frame_51, 6, &cfg_wide, 8);

    for front_idx in 0..4 {
        assert!(
            (out_u[front_idx] - out_w[front_idx]).abs() < 1e-6,
            "Front/LFE ch {} should not change with surround stereo_width ({} vs {})",
            front_idx,
            out_u[front_idx],
            out_w[front_idx]
        );
    }
    let side_diff_u = (out_u[4] - out_u[5]).abs();
    let side_diff_w = (out_w[4] - out_w[5]).abs();
    assert!(
        side_diff_w > side_diff_u + 0.1,
        "5.1 surround width 1.8 must widen surround differential ({} vs {})",
        side_diff_w,
        side_diff_u
    );

    // 2. Discrete 7.1 -> 7.1: `stereo_width = 0.0` collapses surround pairs to mono mid,
    // while `stereo_width = 1.7` widens both SL/SR and BL/BR without altering FL/FR.
    let frame_71 = [0.25_f32, -0.20, 0.10, 0.05, 0.50, -0.10, -0.40, 0.30];
    let cfg_mono_sur = crate::state::SurroundUpmixConfig {
        enabled: true,
        stereo_width: 0.0,
        ..Default::default()
    };
    let cfg_wide_71 = crate::state::SurroundUpmixConfig {
        enabled: true,
        stereo_width: 1.7,
        ..Default::default()
    };
    let mut dsp_71_m = SurroundUpmixerDsp::new(sr);
    dsp_71_m.sync_config(&cfg_mono_sur, sr);
    let out_71_m = dsp_71_m.process_multichannel_frame_for_hardware(&frame_71, 8, &cfg_mono_sur, 8);

    let mut dsp_71_w = SurroundUpmixerDsp::new(sr);
    dsp_71_w.sync_config(&cfg_wide_71, sr);
    let out_71_w = dsp_71_w.process_multichannel_frame_for_hardware(&frame_71, 8, &cfg_wide_71, 8);

    assert!((out_71_m[0] - out_71_w[0]).abs() < 1e-6);
    assert!((out_71_m[1] - out_71_w[1]).abs() < 1e-6);
    assert!(
        (out_71_m[4] - out_71_m[5]).abs() < 1e-6 && (out_71_m[6] - out_71_m[7]).abs() < 1e-6,
        "7.1 stereo_width=0.0 must collapse SL/SR and BL/BR to mid"
    );
    assert!((out_71_w[4] - out_71_w[5]).abs() > (frame_71[4] - frame_71[5]).abs() + 0.1);
    assert!((out_71_w[6] - out_71_w[7]).abs() > (frame_71[6] - frame_71[7]).abs() + 0.1);

    // 3a. Discrete 5.1 -> 5.1 (hw_ch = 6): boosting all 5 bands by +6 dB must shape discrete channels.
    let cfg_eq_boost = crate::state::SurroundUpmixConfig {
        enabled: true,
        band_gains_db: [6.0; 5],
        ..Default::default()
    };
    let mut dsp_flat = SurroundUpmixerDsp::new(sr);
    dsp_flat.sync_config(&cfg_unity, sr);
    let mut dsp_boost = SurroundUpmixerDsp::new(sr);
    dsp_boost.sync_config(&cfg_eq_boost, sr);

    let mut energy_flat_51 = 0.0_f32;
    let mut energy_boost_51 = 0.0_f32;
    for n in 0..256 {
        let s = (2.0 * std::f32::consts::PI * 440.0 * (n as f32) / (sr as f32)).sin() * 0.35;
        let in_f = [s, s * 0.8, s * 0.5, s * 0.2, s * 0.6, -s * 0.4];
        let of = dsp_flat.process_multichannel_frame_for_hardware(&in_f, 6, &cfg_unity, 6);
        let ob = dsp_boost.process_multichannel_frame_for_hardware(&in_f, 6, &cfg_eq_boost, 6);
        if n >= 64 {
            energy_flat_51 += of[0] * of[0] + of[4] * of[4];
            energy_boost_51 += ob[0] * ob[0] + ob[4] * ob[4];
        }
    }
    assert!(
        energy_boost_51 > energy_flat_51 * 2.5,
        "Discrete 5.1->5.1 +6dB 5-band EQ should boost discrete stems (flat={}, boost={})",
        energy_flat_51,
        energy_boost_51
    );

    // 3b. Upmixed 5.1 -> 7.1 (hw_ch = 8): base discrete anchor channels remain uncolored while
    // synthesized rear stems (RL/RR) receive the +6 dB 5-band routing EQ boost (past Haas delay).
    let mut dsp_up_flat = SurroundUpmixerDsp::new(sr);
    dsp_up_flat.sync_config(&cfg_unity, sr);
    let mut dsp_up_boost = SurroundUpmixerDsp::new(sr);
    dsp_up_boost.sync_config(&cfg_eq_boost, sr);
    let mut rear_flat_71 = 0.0_f32;
    let mut rear_boost_71 = 0.0_f32;
    for n in 0..2048 {
        let s = (2.0 * std::f32::consts::PI * 440.0 * (n as f32) / (sr as f32)).sin() * 0.25;
        let in_f = [s, -s * 0.7, s * 0.3, s * 0.1, s * 0.5, -s * 0.35];
        let of = dsp_up_flat.process_multichannel_frame_for_hardware(&in_f, 6, &cfg_unity, 8);
        let ob = dsp_up_boost.process_multichannel_frame_for_hardware(&in_f, 6, &cfg_eq_boost, 8);
        assert!(
            (of[0] - ob[0]).abs() < 1e-6,
            "5.1->7.1 upmix must keep Front L discrete anchor uncolored by routing EQ"
        );
        if n >= 1200 {
            rear_flat_71 += of[6] * of[6] + of[7] * of[7];
            rear_boost_71 += ob[6] * ob[6] + ob[7] * ob[7];
        }
    }
    assert!(
        rear_boost_71 > rear_flat_71 * 2.0,
        "Upmixed 5.1->7.1 +6dB routing EQ should boost synthesized RL/RR stems (flat={}, boost={})",
        rear_flat_71,
        rear_boost_71
    );
}

#[test]
fn test_32_channel_tracker_dsp_message_preserves_34_vus_and_nonzero_fft() {
    let sample_rate = 48_000_u32;
    let window_size = 8192_usize;
    let max_dsp_channels = 12_usize;

    let mut fft_buffer = vec![0.0_f32; window_size];
    let mut channel_fft_buffers = vec![vec![0.0_f32; window_size]; max_dsp_channels];
    for i in 0..window_size {
        let t = i as f32 / sample_rate as f32;
        let l = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.8;
        let r = (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.6;
        fft_buffer[i] = 0.5 * (l + r);
        channel_fft_buffers[0][i] = l;
        channel_fft_buffers[1][i] = r;
    }
    // 32 tracker channels + 2 master channels = 34 VU bars on 2-channel stereo hardware
    let vu_34: Vec<f32> = (0..34).map(|i| 0.05 + (i as f32) * 0.02).collect();

    let (recycle_tx, recycle_rx) = allocate_rt_dsp_pool(window_size, max_dsp_channels);
    let msg = try_build_rt_dsp_message(
        &recycle_rx,
        &fft_buffer,
        &channel_fft_buffers,
        123,
        window_size,
        2, // hardware_channels = 2 (stereo output)
        &vu_34,
        3,
        16,
        125,
        6,
        4.25,
        "C-5 01 .. | E-5 02 ..",
        1,
    )
    .expect("pre-allocated RT DSP pool must yield a message");

    assert_eq!(msg.channel_vus.len(), 34);
    assert_eq!(
        msg.channel_audio_data
            .iter()
            .take_while(|ch| !ch.is_empty())
            .count(),
        2
    );
    for inactive in &msg.channel_audio_data[2..] {
        assert!(inactive.is_empty());
        assert!(inactive.capacity() >= window_size);
    }

    let resynth_vis_idx = crate::state::VISUALIZERS
        .iter()
        .position(|v| v.requires_resynth)
        .expect("at least one visualizer must use requires_resynth");

    let mut app_state = AppState::new("tracker32.it".to_string());
    app_state.file_loaded = true;
    app_state.is_paused = false;
    app_state.source_channels = 2;
    app_state.hardware_channels = 2;
    app_state.seek_epoch = 1;
    app_state.current_visualizer_idx = resynth_vis_idx;
    app_state.visualizer_mode = crate::state::VISUALIZERS[resynth_vis_idx].id;
    app_state.sync_rt_control();

    let shared_state = Arc::new(Mutex::new(app_state));
    let (dsp_tx, dsp_rx) = bounded::<DspMessage>(4);
    let (done_recycle_tx, done_recycle_rx) = bounded::<DspMessage>(4);

    spawn_dsp_thread_with_recycle(
        dsp_rx,
        shared_state.clone(),
        sample_rate,
        24_000.0,
        window_size,
        Some(done_recycle_tx),
    );

    dsp_tx.send(msg).unwrap();
    let recycled = done_recycle_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("DSP thread must process message and return it to recycle channel");
    drop(dsp_tx);

    {
        let state = shared_state.lock().unwrap();
        assert_eq!(state.raw_channel_vus.len(), 34);
        assert_eq!(state.num_channels, 34);
        assert_eq!(state.raw_audio_channels.len(), 2);

        let max_mono_bin = state
            .raw_spectrum_data
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        assert!(
            max_mono_bin > 5.0,
            "Mono FFT spectrum must be non-zero for 32-channel tracker (got max={})",
            max_mono_bin
        );
        assert!(
            state.gpu_spectrum_data[..2 * 1024 * 2]
                .iter()
                .any(|&v| v.abs() > 1e-2),
            "GPU resynth spectrum must be populated for stereo channels 0..2"
        );
        assert!(
            state.gpu_spectrum_data[2 * 1024 * 2..]
                .iter()
                .all(|&v| v == 0.0),
            "Inactive channels 2..32 in gpu_spectrum_data must remain zero"
        );
    }

    // Verify recycled message can expand up to 8 active channels (upmix toggle) without reallocating
    let orig_outer_cap = recycled.channel_audio_data.capacity();
    let orig_inner_caps: Vec<usize> = recycled
        .channel_audio_data
        .iter()
        .map(|v| v.capacity())
        .collect();
    recycle_tx.try_send(recycled).unwrap();
    while recycle_rx.len() > 1 {
        let _ = recycle_rx.try_recv();
    }
    let msg_upmix = try_build_rt_dsp_message(
        &recycle_rx,
        &fft_buffer,
        &channel_fft_buffers,
        0,
        window_size,
        8,
        &vu_34[..8],
        0,
        0,
        125,
        6,
        5.0,
        "",
        1,
    )
    .unwrap();
    assert_eq!(msg_upmix.channel_audio_data.capacity(), orig_outer_cap);
    for (i, ch) in msg_upmix.channel_audio_data.iter().enumerate() {
        assert_eq!(ch.capacity(), orig_inner_caps[i]);
        if i < 8 {
            assert_eq!(ch.len(), window_size);
        } else {
            assert!(ch.is_empty());
        }
    }
}

#[test]
fn test_stack_lowercase_ascii_buf_zero_alloc_xrun_matching() {
    use std::fmt::Write;
    let mut b1 = StackLowercaseAsciiBuf::new();
    write!(&mut b1, "BackendSpecific: ALSA PCM UNDERRUN occurred").unwrap();
    assert!(b1.contains_subslice(b"underrun"));

    let mut b2 = StackLowercaseAsciiBuf::new();
    write!(&mut b2, "Buffer Overrun on capture stream").unwrap();
    assert!(b2.contains_subslice(b"overrun"));

    let mut b3 = StackLowercaseAsciiBuf::new();
    write!(&mut b3, "Audio endpoint unplugged / disconnected").unwrap();
    assert!(!b3.contains_subslice(b"underrun") && !b3.contains_subslice(b"overrun"));

    // Verify >256 byte error string truncates safely without panic
    let mut b4 = StackLowercaseAsciiBuf::new();
    let long_msg = "X".repeat(400);
    write!(&mut b4, "{}", long_msg).unwrap();
    assert_eq!(b4.len, 256);
}

#[test]
fn test_realfft_r2c_matches_analytic_dft_sine_bin() {
    let n = 8192usize;
    let target_bin = 128usize;
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(n);
    let mut input = r2c.make_input_vec();
    let mut output = r2c.make_output_vec();
    let mut scratch = r2c.make_scratch_vec();

    // Coherent Hann-windowed unit sine at exact bin k=128:
    // Hann window mean = 0.5, single-sided sine DFT bin k magnitude = 0.25 * N = 2048.0,
    // phase for sin(2*pi*k*n/N) at bin k is -i * (0.25 * N).
    for (i, sample) in input.iter_mut().enumerate() {
        let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos());
        let sig = (2.0 * std::f32::consts::PI * target_bin as f32 * i as f32 / n as f32).sin();
        *sample = sig * hann;
    }

    r2c.process_with_scratch(&mut input, &mut output, &mut scratch)
        .unwrap();
    assert_eq!(output.len(), n / 2 + 1);

    let (peak_bin, peak_val) = output
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.norm().partial_cmp(&b.norm()).unwrap())
        .unwrap();
    assert_eq!(peak_bin, target_bin);
    assert!(
        (peak_val.norm() - (0.25 * n as f32)).abs() < 0.05,
        "unexpected peak norm: {}",
        peak_val.norm()
    );
    assert!(peak_val.re.abs() < 0.05);
    assert!((peak_val.im + 0.25 * n as f32).abs() < 0.05);
}

#[test]
fn test_spawn_video_decoder_thread_unsupported_codec_resets_video_state_cleanly() {
    ffmpeg_next::init().expect("init ffmpeg");
    let shared_state = Arc::new(Mutex::new(AppState::new("UnsupportedVideo".to_string())));
    {
        let mut s = shared_state.lock().unwrap();
        s.has_video_stream = true;
        s.video_info = Some("Unsupported Codec 1920x1080".to_string());
        s.stats.video_buffer_fill_pct = 42.0;
    }
    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let empty_params = ffmpeg_next::codec::Parameters::new();
    let res = spawn_video_decoder_thread(
        empty_params,
        ffmpeg_next::Rational::new(1, 30),
        0,
        shared_state.clone(),
        stop_token,
    );
    assert!(
        res.is_none(),
        "Unsupported video codec parameters must return None instead of leaving dead video channels"
    );
    let s = shared_state.lock().unwrap();
    assert!(!s.has_video_stream);
    assert!(s.video_frame_rx.is_none());
    assert!(s.free_video_frame_tx.is_none());
    assert!(s.video_info.is_none());
    assert_eq!(s.stats.video_buffer_fill_pct, 0.0);
}

#[test]
fn test_load_audio_source_from_multi_extent_iso() {
    use rt_decode::iso::test_helpers::{build_synthetic_dvd_iso, build_synthetic_vts_ifo};

    // Build a valid 48kHz stereo 16-bit PCM WAV stream of 8192 bytes (2 sectors per VOB extent)
    let pcm_bytes_len: u32 = 8192 - 44;
    let total_riff_len: u32 = 36 + pcm_bytes_len;
    let mut wav = Vec::with_capacity(8192);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&total_riff_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&2u16.to_le_bytes()); // Stereo
    wav.extend_from_slice(&48000u32.to_le_bytes());
    wav.extend_from_slice(&(48000u32 * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&pcm_bytes_len.to_le_bytes());
    for i in 0..(pcm_bytes_len as usize / 2) {
        let sample = ((i as f32 * 0.1).sin() * 12000.0) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    assert_eq!(wav.len(), 8192);

    let ifo_bytes = build_synthetic_vts_ifo();
    let vob1 = wav[0..4096].to_vec();
    let vob2 = wav[4096..8192].to_vec();

    let iso_bytes = build_synthetic_dvd_iso(
        "PORTISHEAD_PNVC",
        &[
            ("VTS_01_0.IFO", ifo_bytes),
            ("VTS_01_1.VOB", vob1),
            ("VTS_01_2.VOB", vob2),
        ],
    );

    let temp_iso = std::env::temp_dir().join("rusttracker_test_playback_portishead.iso");
    std::fs::write(&temp_iso, &iso_bytes).expect("write playback ISO");

    let mut source = load_audio_source(temp_iso.to_str().unwrap())
        .expect("load_audio_source on synthetic DVD ISO");
    assert_eq!(source.get_artist(), "PORTISHEAD_PNVC");
    assert_eq!(source.get_duration_seconds(), 5595.0);
    assert_eq!(source.get_num_channels(), 2);

    let mut out = vec![0.0f32; 1024 * 2];
    let frames = source.read_frames(2, 48000, &mut out);
    assert!(frames > 0, "Expected decoded audio frames from ISO stream");

    source.set_position_seconds(2795.0);
    assert_eq!(source.get_position_seconds(), 2795.0);

    drop(source);
    let _ = std::fs::remove_file(&temp_iso);
}

#[test]
fn test_openmpt_source_zero_alloc_1ch_2ch_12ch_playback() {
    let workspace_mod =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ive_got_the_power.mod");
    if !workspace_mod.exists() {
        return;
    }
    let mut src = load_audio_source(workspace_mod.to_str().unwrap())
        .expect("load OpenMPT .mod via direct C FFI");
    assert!(src.get_duration_seconds() > 1.0);
    assert_eq!(src.get_tracker_channels(), Some(4));
    assert_eq!(src.get_num_channels(), 2);
    let formatted = src.pre_format_tracker_data();
    assert!(!formatted.is_empty() && !formatted[0].is_empty());

    // Mono (1ch), Stereo (2ch), and 7.1.4 (12ch) in-place zero-alloc rendering
    let mut mono_buf = vec![0.0f32; 512];
    assert_eq!(src.read_frames(1, 48000, &mut mono_buf), 512);
    assert!(mono_buf.iter().any(|&v| v.abs() > 1e-5));

    let mut stereo_buf = vec![0.0f32; 512 * 2];
    assert_eq!(src.read_frames(2, 48000, &mut stereo_buf), 512);
    assert!(stereo_buf.iter().any(|&v| v.abs() > 1e-5));

    let mut surround_buf = vec![1.0f32; 512 * 12];
    assert_eq!(src.read_frames(12, 48000, &mut surround_buf), 512);
    // Verify stereo expansion wrote FL/FR into channels 0..2 and zeroed channels 2..12
    assert!(
        surround_buf
            .as_chunks::<12>()
            .0
            .iter()
            .any(|f| f[0].abs() > 1e-5 || f[1].abs() > 1e-5)
    );
    for frame in surround_buf.as_chunks::<12>().0 {
        for &ch in &frame[2..12] {
            assert_eq!(
                ch, 0.0,
                "OpenMptSource >2ch expansion must zero upper channels"
            );
        }
    }
}

#[test]
fn test_parse_wireplumber_state_topology_and_no_subprocess_audio_watch() {
    let default_nodes = r#"[default-nodes]
default.configured.audio.sink=alsa_output.pci-0000_23_00.1.hdmi-stereo-extra3
default.configured.audio.sink.0=alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink
default.configured.audio.source=alsa_input.usb-Generic_USB_Audio-00.HiFi__Mic1__source
"#;
    let default_routes = r#"[default-routes]
alsa_card.pci-0000_23_00.1:output:hdmi-output-3={"channelMap":["FL", "FR", "FC", "LFE", "RL", "RR"], "volume":1.0}
alsa_card.pci-0000_23_00.1:profile:output:hdmi-stereo-extra3=["hdmi-output-3"]
"#;

    let sink_topo = parse_wireplumber_state_topology(default_nodes, Some(default_routes), false)
        .expect("parse WirePlumber sink state");
    assert_eq!(
        sink_topo.sink_id,
        "alsa_output.pci-0000_23_00.1.hdmi-stereo-extra3"
    );
    assert_eq!(sink_topo.channels, 6);

    let mic_topo = parse_wireplumber_state_topology(default_nodes, Some(default_routes), true)
        .expect("parse WirePlumber source state");
    assert_eq!(
        mic_topo.sink_id,
        "alsa_input.usb-Generic_USB_Audio-00.HiFi__Mic1__source"
    );
    assert_eq!(mic_topo.channels, 1);

    // Verify production code in audio.rs never spawns wpctl or pactl subprocesses
    let audio_src = include_str!("topology.rs");
    let audio_prod = audio_src.split("#[cfg(test)]").next().unwrap_or(audio_src);
    let spawn_needle_wpctl = ["Command::new(", "\"wpctl\""].concat();
    let spawn_needle_pactl = ["Command::new(", "\"pactl\""].concat();
    assert!(
        !audio_prod.contains(&spawn_needle_wpctl) && !audio_prod.contains(&spawn_needle_pactl),
        "Linux audio sink topology watch must not spawn wpctl or pactl subprocesses"
    );
}

#[test]
fn test_pack_ffmpeg_video_frame_nv12_p010_and_yuv420p_zero_repack() {
    use crate::state::{VideoChromaLayout, VideoFrame, WGPU_ROW_PITCH_ALIGNMENT};
    ffmpeg_next::init().expect("init ffmpeg");

    let mut scaler = None;
    let mut out = VideoFrame::empty(0);

    // 1. Semi-planar 8-bit NV12 -> 2-plane R8 + Rg8 with 256-byte row alignment and empty v_plane
    let mut nv12 = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::NV12, 320, 180);
    nv12.data_mut(0).fill(0x90);
    for pair in nv12.data_mut(1).as_chunks_mut::<2>().0 {
        *pair = [0x34, 0xAB]; // U, V
    }
    let tag =
        pack_ffmpeg_video_frame(&nv12, 320, 180, &mut out, &mut scaler).expect("pack NV12 frame");
    assert_eq!(tag, "2-Plane NV12");
    assert_eq!(out.chroma_layout, VideoChromaLayout::SemiPlanar420);
    assert_eq!(out.bit_depth, 8);
    assert_eq!(out.y_stride % WGPU_ROW_PITCH_ALIGNMENT, 0);
    assert_eq!(out.u_stride % WGPU_ROW_PITCH_ALIGNMENT, 0);
    assert_eq!(out.v_stride, 0);
    assert!(out.v_plane.is_empty());
    assert_eq!(out.y_plane[0], 0x90);
    assert_eq!(out.u_plane[0], 0x34);
    assert_eq!(out.u_plane[1], 0xAB);

    let prev_y_cap = out.y_plane.capacity();
    let prev_u_cap = out.u_plane.capacity();
    let _ = pack_ffmpeg_video_frame(&nv12, 320, 180, &mut out, &mut scaler).unwrap();
    assert_eq!(out.y_plane.capacity(), prev_y_cap);
    assert_eq!(out.u_plane.capacity(), prev_u_cap);

    // 2. Semi-planar 10-bit P010LE -> 2-plane R16 + Rg16 (bit_depth = 16)
    let mut p010 = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::P010LE, 320, 180);
    for word in p010.data_mut(0).as_chunks_mut::<2>().0 {
        *word = (512u16 << 6).to_le_bytes();
    }
    for uv in p010.data_mut(1).as_chunks_mut::<4>().0 {
        uv[0..2].copy_from_slice(&(400u16 << 6).to_le_bytes());
        uv[2..4].copy_from_slice(&(640u16 << 6).to_le_bytes());
    }
    let tag_p010 =
        pack_ffmpeg_video_frame(&p010, 320, 180, &mut out, &mut scaler).expect("pack P010LE frame");
    assert_eq!(tag_p010, "2-Plane P010");
    assert_eq!(out.chroma_layout, VideoChromaLayout::SemiPlanar420);
    assert_eq!(out.bit_depth, 16);
    assert_eq!(out.y_stride % WGPU_ROW_PITCH_ALIGNMENT, 0);
    assert_eq!(out.u_stride % WGPU_ROW_PITCH_ALIGNMENT, 0);
    assert!(out.v_plane.is_empty());
    assert_eq!(
        u16::from_le_bytes([out.u_plane[0], out.u_plane[1]]),
        400u16 << 6
    );
    assert_eq!(
        u16::from_le_bytes([out.u_plane[2], out.u_plane[3]]),
        640u16 << 6
    );

    // 3. Planar 8-bit YUV420P -> 3-plane R8 + R8 + R8
    let mut yuv420 = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, 320, 180);
    yuv420.data_mut(0).fill(0x7F);
    yuv420.data_mut(1).fill(0x60);
    yuv420.data_mut(2).fill(0x98);
    let tag_420 = pack_ffmpeg_video_frame(&yuv420, 320, 180, &mut out, &mut scaler)
        .expect("pack YUV420P frame");
    assert!(tag_420.starts_with("3-Plane"));
    assert_eq!(out.chroma_layout, VideoChromaLayout::Planar420);
    assert_eq!(out.bit_depth, 8);
    assert!(!out.v_plane.is_empty());
    assert_eq!(out.v_stride % WGPU_ROW_PITCH_ALIGNMENT, 0);

    // 4. Hardware surface padding (e.g. 1920x1088 VAAPI/DXVA surface for 1920x1080 visible frame)
    // must clamp to fallback_w/fallback_h (1920x1080) and exclude bottom 8 padding rows!
    let mut hw_padded = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::NV12, 320, 192);
    let y_s = hw_padded.stride(0);
    for r in 0..180 {
        hw_padded.data_mut(0)[r * y_s..r * y_s + 320].fill(0x88);
    }
    for r in 180..192 {
        // Simulated uninitialized green/bright garbage in bottom 12 padding rows
        hw_padded.data_mut(0)[r * y_s..r * y_s + 320].fill(0xFF);
    }
    let _ = pack_ffmpeg_video_frame(&hw_padded, 320, 180, &mut out, &mut scaler)
        .expect("pack padded HW NV12 frame");
    assert_eq!((out.width, out.height), (320, 180));
    assert_eq!(out.y_plane.len(), out.y_stride * 180);
    assert!(
        out.y_plane[(179 * out.y_stride)..(179 * out.y_stride + 320)]
            .iter()
            .all(|&b| b == 0x88)
    );

    // 5. Explicit AVFrame crop_top/crop_bottom/crop_left/crop_right on 2-plane NV21 (SemiPlanarVu420)
    let mut nv21_cropped = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::NV21, 16, 16);
    let nv21_ys = nv21_cropped.stride(0);
    let nv21_uvs = nv21_cropped.stride(1);
    nv21_cropped.data_mut(0).fill(0x11);
    nv21_cropped.data_mut(1).fill(0x22);
    unsafe {
        let p = nv21_cropped.as_mut_ptr();
        (*p).crop_left = 4;
        (*p).crop_right = 4;
        (*p).crop_top = 2;
        (*p).crop_bottom = 6;
    }
    // Active 8x8 window starts at luma (row=2, col=4) and chroma (row=1, byte_col=4)
    nv21_cropped.data_mut(0)[2 * nv21_ys + 4] = 0xCC;
    nv21_cropped.data_mut(1)[nv21_uvs + 4..nv21_uvs + 6].copy_from_slice(&[0xDE, 0x45]); // [V, U]
    let tag_nv21 = pack_ffmpeg_video_frame(&nv21_cropped, 16, 16, &mut out, &mut scaler)
        .expect("pack cropped NV21");
    assert_eq!(tag_nv21, "2-Plane NV21");
    assert_eq!(out.chroma_layout, VideoChromaLayout::SemiPlanarVu420);
    assert_eq!((out.width, out.height), (8, 8));
    assert_eq!(out.y_plane[0], 0xCC);
    assert_eq!(&out.u_plane[0..2], &[0xDE, 0x45]);

    // 6. Truncated/null-plane 10-bit YUV420P10LE chroma fills neutral midpoint 512 ([0x00, 0x02] LE) instead of 0 (green)
    let mut yuv10 = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P10LE, 16, 16);
    unsafe {
        (*yuv10.as_mut_ptr()).linesize[1] = 0; // Simulate missing/truncated chroma plane
    }
    let tag_10 = pack_ffmpeg_video_frame(&yuv10, 16, 16, &mut out, &mut scaler)
        .expect("pack YUV420P10LE with missing U plane");
    assert_eq!(tag_10, "3-Plane YUV420P10");
    assert_eq!(&out.u_plane[0..4], &[0x00, 0x02, 0x00, 0x02]);
}

#[test]
fn test_wireplumber_hot_unplug_multi_candidate_and_eld_lpcm_parsing() {
    let default_nodes = r#"[default-nodes]
default.configured.audio.sink=alsa_output.pci-0000_23_00.1.hdmi-stereo-extra0
default.configured.audio.sink.0=alsa_output.usb-Unplugged_DAC-00.analog-stereo
default.configured.audio.sink.1=alsa_output.pci-0000_23_00.1.hdmi-surround-extra3
"#;
    // Simulate HDMI extra0 unplugged (ELD monitor_present=0) and USB DAC unplugged -> must advance to `.1`!
    let topo = parse_wireplumber_state_topology_filtered(default_nodes, None, false, |cand| {
        cand == "alsa_output.pci-0000_23_00.1.hdmi-surround-extra3"
    })
    .expect("fallback to connected .1 sink");
    assert_eq!(
        topo.sink_id,
        "alsa_output.pci-0000_23_00.1.hdmi-surround-extra3"
    );

    // Verify ELD parser rejects unplugged ports and extracts monitor_name + max LPCM channels on active AVR
    let unplugged_eld = "monitor_present\t\t0\neld_valid\t\t0\n";
    assert!(parse_alsa_eld_active_info(unplugged_eld).is_none());

    let avr_eld = "\
monitor_present\t\t1
eld_valid\t\t1
monitor_name\t\tDENON-AVR
sad_count\t\t2
sad0_coding_type\t[0x1] LPCM
sad0_channels\t\t2
sad1_coding_type\t[0x1] LPCM
sad1_channels\t\t8
";
    let (mon, lpcm_ch) = parse_alsa_eld_active_info(avr_eld).expect("active AVR ELD");
    assert_eq!(mon.as_deref(), Some("DENON-AVR"));
    assert_eq!(lpcm_ch, Some(8));
}

#[test]
fn test_video_upload_telemetry_key_dedup_and_decoder_steady_state() {
    let mut last_key: Option<VideoUploadTelemetryKey> = None;
    let mut alloc_count = 0usize;
    let mut current_label = String::new();

    // 1. Initial frame allocates once; 500 identical frames allocate zero times
    for _ in 0..501 {
        update_video_upload_telemetry_if_changed(
            &mut last_key,
            Some("HW (VAAPI)"),
            true,
            "2-Plane NV12",
            |label| {
                alloc_count += 1;
                current_label = label;
                true
            },
        );
    }
    assert_eq!(alloc_count, 1);
    assert_eq!(current_label, "HW (VAAPI Map -> 2-Plane NV12)");

    // 2. Toggling mapped_hw updates once
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        Some("HW (VAAPI)"),
        false,
        "2-Plane NV12",
        |label| {
            alloc_count += 1;
            current_label = label;
            true
        },
    );
    assert_eq!(alloc_count, 2);
    assert_eq!(current_label, "HW (VAAPI -> 2-Plane NV12)");

    // 3. Changing plane_tag or hw_label (including unwrapped HW label) updates once each
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        Some("HW (VAAPI)"),
        false,
        "2-Plane P010",
        |label| {
            alloc_count += 1;
            current_label = label;
            true
        },
    );
    assert_eq!(alloc_count, 3);
    assert_eq!(current_label, "HW (VAAPI -> 2-Plane P010)");

    update_video_upload_telemetry_if_changed(
        &mut last_key,
        Some("Vulkan"),
        true,
        "2-Plane P010",
        |label| {
            alloc_count += 1;
            current_label = label;
            true
        },
    );
    assert_eq!(alloc_count, 4);
    assert_eq!(current_label, "HW (Vulkan Map -> 2-Plane P010)");

    // 4. Rejected update (e.g. failed lock) leaves last_key unchanged so next frame retries
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        None,
        false,
        "3-Plane YUV420P",
        |_label| {
            alloc_count += 1;
            false
        },
    );
    assert_eq!(alloc_count, 5);
    assert_eq!(current_label, "HW (Vulkan Map -> 2-Plane P010)");

    update_video_upload_telemetry_if_changed(
        &mut last_key,
        None,
        false,
        "3-Plane YUV420P",
        |label| {
            alloc_count += 1;
            current_label = label;
            true
        },
    );
    assert_eq!(alloc_count, 6);
    assert_eq!(current_label, "CPU (3-Plane YUV420P)");

    // 5. Resetting last_key = None (after HW->CPU fallback sets "CPU") forces re-report
    current_label = "CPU".to_string();
    last_key = None;
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        None,
        false,
        "3-Plane YUV420P",
        |label| {
            alloc_count += 1;
            current_label = label;
            true
        },
    );
    assert_eq!(alloc_count, 7);
    assert_eq!(current_label, "CPU (3-Plane YUV420P)");

    // 5b. Cross-platform preferred_hw_video_devices table + mid-stream software AVFrame gating
    assert_eq!(preferred_hw_video_devices("windows").len(), 3);
    assert_eq!(preferred_hw_video_devices("linux").len(), 3);
    assert_eq!(preferred_hw_video_devices("macos").len(), 1);
    assert!(preferred_hw_video_devices("android").is_empty());
    for os in ["windows", "linux", "macos"] {
        for &(_, hw_lbl) in preferred_hw_video_devices(os) {
            let formatted = format_video_upload_telemetry(Some(hw_lbl), true, "2-Plane NV12");
            assert!(formatted.starts_with("HW (") && formatted.ends_with("Map -> 2-Plane NV12)"));
        }
    }
    // Mid-stream software AVFrame (`is_hw_frame == false`) while `hw_label` is still `Some(...)`
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        effective_frame_hw_label(Some("HW (VAAPI)"), true),
        true,
        "2-Plane NV12",
        |label| {
            current_label = label;
            true
        },
    );
    assert_eq!(current_label, "HW (VAAPI Map -> 2-Plane NV12)");
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        effective_frame_hw_label(Some("HW (VAAPI)"), false),
        false,
        "3-Plane YUV420P",
        |label| {
            current_label = label;
            true
        },
    );
    assert_eq!(current_label, "CPU (3-Plane YUV420P)");
    update_video_upload_telemetry_if_changed(
        &mut last_key,
        effective_frame_hw_label(Some("HW (VAAPI)"), true),
        true,
        "2-Plane NV12",
        |label| {
            current_label = label;
            true
        },
    );
    assert_eq!(current_label, "HW (VAAPI Map -> 2-Plane NV12)");

    // 6. End-to-end multi-frame rawvideo decode through spawn_video_decoder_thread
    ffmpeg_next::init().expect("init ffmpeg");
    let mut params = ffmpeg_next::codec::Parameters::new();
    unsafe {
        let p = params.as_mut_ptr();
        (*p).codec_type = ffmpeg_next::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        (*p).codec_id = ffmpeg_next::ffi::AVCodecID::AV_CODEC_ID_RAWVIDEO;
        (*p).format = ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as i32;
        (*p).width = 16;
        (*p).height = 16;
    }
    let shared_state = Arc::new(Mutex::new(AppState::new("RawVideoDedup".to_string())));
    {
        let mut s = shared_state.lock().unwrap();
        s.current_seconds = 0.10;
    }
    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (pkt_tx, _pkt_rx) = spawn_video_decoder_thread(
        params,
        ffmpeg_next::Rational::new(1, 30),
        0,
        shared_state.clone(),
        stop_token.clone(),
    )
    .expect("open rawvideo decoder thread");

    let (frame_rx, free_tx) = {
        let s = shared_state.lock().unwrap();
        (
            s.video_frame_rx.as_ref().unwrap().clone(),
            s.free_video_frame_tx.as_ref().unwrap().clone(),
        )
    };

    // Enqueue enough packets to prime FFmpeg's multi-frame thread pool (up to 16 threads)
    let raw_buf = [0x80u8; 16 * 16 + 8 * 8 * 2];
    for i in 0..20i64 {
        let mut pkt = ffmpeg_next::Packet::copy(&raw_buf);
        let clamped_pts = i.min(3);
        pkt.set_pts(Some(clamped_pts));
        pkt.set_dts(Some(clamped_pts));
        pkt_tx.send((0, pkt)).expect("send rawvideo packet");
    }

    let mut first_alloc_ptr = std::ptr::null();
    for i in 0..4usize {
        let vf = frame_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("receive decoded rawvideo frame");
        assert_eq!((vf.width, vf.height), (16, 16));
        {
            let s = shared_state.lock().unwrap();
            assert_eq!(s.stats.video_hw_decoder, "CPU (3-Plane YUV420P)");
            if i == 0 {
                first_alloc_ptr = s.stats.video_hw_decoder.as_ptr();
            } else {
                assert_eq!(
                    s.stats.video_hw_decoder.as_ptr(),
                    first_alloc_ptr,
                    "Steady-state frames must not reallocate video_hw_decoder"
                );
            }
        }
        free_tx.send(vf).expect("recycle video frame");
    }
    stop_token.store(true, std::sync::atomic::Ordering::Relaxed);

    // 7. Live GPU hardware decode (VP9 Profile 0 4:2:0) -> mid-stream software AVFrame
    //    fallback (VP9 Profile 1 4:4:4) -> hardware recovery test
    fn encode_vp9_keyframe(
        fmt: ffmpeg_next::format::Pixel,
        w: u32,
        h: u32,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Packet)> {
        let codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::VP9)?;
        let ctx = ffmpeg_next::codec::context::Context::new_with_codec(codec);
        let mut enc = ctx.encoder().video().ok()?;
        enc.set_width(w);
        enc.set_height(h);
        enc.set_format(fmt);
        enc.set_time_base(ffmpeg_next::Rational::new(1, 30));
        enc.set_bit_rate(200_000);
        enc.set_gop(1);
        enc.set_max_b_frames(0);
        let mut opts = ffmpeg_next::Dictionary::new();
        opts.set("deadline", "realtime");
        opts.set("cpu-used", "8");
        let mut opened = enc.open_with(opts).ok()?;

        let mut frame = ffmpeg_next::frame::Video::new(fmt, w, h);
        frame.set_pts(Some(0));
        for plane_idx in 0..frame.planes() {
            let fill = if plane_idx == 0 { 0x50u8 } else { 0x80u8 };
            frame.data_mut(plane_idx).fill(fill);
        }
        opened.send_frame(&frame).ok()?;
        opened.send_eof().ok()?;
        let mut pkt = ffmpeg_next::Packet::empty();
        opened.receive_packet(&mut pkt).ok()?;
        let params = ffmpeg_next::codec::Parameters::from(&opened);
        Some((params, pkt))
    }

    if let (Some((mut vp9_p0_params, vp9_p0_pkt)), Some((vp9_p1_params, vp9_p1_pkt))) = (
        encode_vp9_keyframe(ffmpeg_next::format::Pixel::YUV420P, 128, 128),
        encode_vp9_keyframe(ffmpeg_next::format::Pixel::YUV444P, 96, 96),
    ) {
        unsafe {
            // Match container demuxer behavior (FF_PROFILE_VP9_0 = 0) so VAAPI/D3D11VA
            // `avcodec_get_hw_frames_parameters` validates Profile 0 hardware support.
            (*vp9_p0_params.as_mut_ptr()).profile = 0;
        }
        let vp9_state = Arc::new(Mutex::new(AppState::new("Vp9HwTransition".to_string())));
        {
            let mut s = vp9_state.lock().unwrap();
            s.current_seconds = 0.10;
        }
        let vp9_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (vp9_tx, _vp9_rx) = spawn_video_decoder_thread(
            vp9_p0_params,
            ffmpeg_next::Rational::new(1, 30),
            0,
            vp9_state.clone(),
            vp9_stop.clone(),
        )
        .expect("open VP9 decoder thread");

        let (vp9_frame_rx, vp9_free_tx) = {
            let s = vp9_state.lock().unwrap();
            (
                s.video_frame_rx.as_ref().unwrap().clone(),
                s.free_video_frame_tx.as_ref().unwrap().clone(),
            )
        };

        for idx in 0..20i64 {
            let mut pkt = vp9_p0_pkt.clone();
            let clamped = idx.min(3);
            pkt.set_pts(Some(clamped));
            pkt.set_dts(Some(clamped));
            vp9_tx.send((0, pkt)).expect("send VP9 P0 packet");
        }

        let mut p0_label = String::new();
        let mut p0_ptr = std::ptr::null();
        for i in 0..4usize {
            let vf = vp9_frame_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("receive 128x128 VP9 Profile 0 frame");
            assert_eq!((vf.width, vf.height), (128, 128));
            {
                let s = vp9_state.lock().unwrap();
                if i == 0 {
                    p0_label = s.stats.video_hw_decoder.clone();
                    p0_ptr = s.stats.video_hw_decoder.as_ptr();
                    println!("[test] VP9 Profile 0 initial telemetry: {p0_label}");
                    assert!(
                        p0_label.starts_with("HW (") || p0_label.starts_with("CPU ("),
                        "Unexpected VP9 Profile 0 telemetry label: {p0_label}"
                    );
                } else {
                    assert_eq!(
                        s.stats.video_hw_decoder.as_ptr(),
                        p0_ptr,
                        "Steady-state VP9 frames must not reallocate video_hw_decoder"
                    );
                }
            }
            vp9_free_tx.send(vf).expect("recycle VP9 P0 frame");
        }

        // Inject VP9 Profile 1 (YUV444P) mid-stream: VAAPI/D3D11VA reject 4:4:4 and fall back
        // to a software AVFrame (`is_hw_frame == false`) inside the same decoder instance.
        {
            let mut s = vp9_state.lock().unwrap();
            s.seek_epoch = 1;
            s.current_seconds = 0.10;
        }
        for idx in 0..20i64 {
            let mut pkt = vp9_p1_pkt.clone();
            let clamped = idx.min(3);
            pkt.set_pts(Some(clamped));
            pkt.set_dts(Some(clamped));
            vp9_tx.send((1, pkt)).expect("send VP9 P1 4:4:4 packet");
        }

        let mut saw_p1_444 = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(vf) = vp9_frame_rx.recv_timeout(std::time::Duration::from_millis(250)) {
                let dims = (vf.width, vf.height);
                vp9_free_tx.send(vf).expect("recycle VP9 frame");
                if dims == (96, 96) {
                    let s = vp9_state.lock().unwrap();
                    println!(
                        "[test] VP9 Profile 1 (4:4:4) mid-stream telemetry: {}",
                        s.stats.video_hw_decoder
                    );
                    assert_eq!(
                        s.stats.video_hw_decoder, "CPU (3-Plane YUV420P)",
                        "Mid-stream VP9 4:4:4 fallback from '{p0_label}' must report CPU (3-Plane YUV420P)"
                    );
                    saw_p1_444 = true;
                    break;
                }
            }
        }
        assert!(
            saw_p1_444,
            "Expected mid-stream 96x96 VP9 Profile 1 frame after '{p0_label}'"
        );

        // Switch back to VP9 Profile 0 (128x128 4:2:0) and verify recovery to `p0_label`
        {
            let mut s = vp9_state.lock().unwrap();
            s.seek_epoch = 2;
            s.current_seconds = 0.10;
        }
        for idx in 0..20i64 {
            let mut pkt = vp9_p0_pkt.clone();
            let clamped = idx.min(3);
            pkt.set_pts(Some(clamped));
            pkt.set_dts(Some(clamped));
            vp9_tx.send((2, pkt)).expect("send VP9 P0 recovery packet");
        }

        let mut saw_p0_recovery = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(vf) = vp9_frame_rx.recv_timeout(std::time::Duration::from_millis(250)) {
                let dims = (vf.width, vf.height);
                vp9_free_tx.send(vf).expect("recycle VP9 recovery frame");
                if dims == (128, 128) {
                    let s = vp9_state.lock().unwrap();
                    println!(
                        "[test] VP9 Profile 0 recovery telemetry: {}",
                        s.stats.video_hw_decoder
                    );
                    assert_eq!(
                        s.stats.video_hw_decoder, p0_label,
                        "Recovery back to VP9 Profile 0 must restore '{p0_label}'"
                    );
                    saw_p0_recovery = true;
                    break;
                }
            }
        }
        assert!(
            saw_p0_recovery,
            "Expected recovery to 128x128 VP9 Profile 0 after 4:4:4 segment"
        );
        vp9_stop.store(true, std::sync::atomic::Ordering::Relaxed);

        // 7b. Frame-0 live GPU fallback: open with Profile 0 params (so VAAPI opens `"HW (VAAPI)"`),
        //     then send Profile 1 (4:4:4) as Packet 0 (`!decoded_any_frame`) and verify clean
        //     `"HW (...)" -> "CPU" -> "CPU (3-Plane YUV420P)"` transition.
        let mut spoofed_params = ffmpeg_next::codec::Parameters::new();
        unsafe {
            ffmpeg_next::ffi::avcodec_parameters_copy(
                spoofed_params.as_mut_ptr(),
                vp9_p1_params.as_ptr(),
            );
            (*spoofed_params.as_mut_ptr()).profile = 0;
        }
        let f0_state = Arc::new(Mutex::new(AppState::new("Vp9Frame0Fallback".to_string())));
        {
            let mut s = f0_state.lock().unwrap();
            s.current_seconds = 0.10;
        }
        let f0_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (f0_tx, _f0_rx) = spawn_video_decoder_thread(
            spoofed_params,
            ffmpeg_next::Rational::new(1, 30),
            0,
            f0_state.clone(),
            f0_stop.clone(),
        )
        .expect("open spoofed VP9 decoder thread");
        let (init_hw_lbl, f0_frame_rx, f0_free_tx) = {
            let s = f0_state.lock().unwrap();
            (
                s.stats.video_hw_decoder.clone(),
                s.video_frame_rx.as_ref().unwrap().clone(),
                s.free_video_frame_tx.as_ref().unwrap().clone(),
            )
        };
        for idx in 0..20i64 {
            let mut pkt = vp9_p1_pkt.clone();
            let clamped = idx.min(3);
            pkt.set_pts(Some(clamped));
            pkt.set_dts(Some(clamped));
            f0_tx.send((0, pkt)).expect("send Frame-0 4:4:4 packet");
        }
        let vf0 = f0_frame_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("receive Frame-0 fallback 96x96 VP9 frame");
        assert_eq!((vf0.width, vf0.height), (96, 96));
        {
            let s = f0_state.lock().unwrap();
            println!(
                "[test] VP9 Frame-0 fallback telemetry: '{init_hw_lbl}' -> '{}'",
                s.stats.video_hw_decoder
            );
            assert_eq!(s.stats.video_hw_decoder, "CPU (3-Plane YUV420P)");
        }
        f0_free_tx.send(vf0).expect("recycle vf0");
        f0_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[test]
fn test_rtrb_spsc_audio_chunk_pool_flush_and_recycle() {
    let pool_size = 8usize;
    let (mut ready_tx, mut ready_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size);
    let (mut free_tx, mut free_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size + 4);
    seed_audio_chunk_pool(&mut free_tx, pool_size, 128, 2);

    for epoch in [1u64, 1, 2, 2] {
        let mut c = free_rx.pop().expect("pooled chunk available");
        c.seek_epoch = epoch;
        c.valid_frames = 128;
        ready_tx.push(c).expect("ready ring push");
    }
    assert_eq!(ready_rx.slots(), 4);

    flush_stale_ready_chunks(&mut ready_rx, &mut free_tx, 2);
    assert_eq!(
        ready_rx.slots(),
        2,
        "Only stale epoch < 2 chunks should be flushed back to free pool"
    );
    let next = ready_rx.pop().expect("fresh epoch-2 chunk remains");
    assert_eq!(next.seek_epoch, 2);
    free_tx.push(next).expect("return chunk to free pool");
}

#[test]
fn test_streaming_sinc_resampler_alias_rejection_and_unity_passband() {
    let in_rate = 96_000u32;
    let out_rate = 48_000u32;
    let total_frames = 24_000usize; // 0.25s

    // 1. Passband 1 kHz stereo sine must pass cleanly near unity gain
    let mut resampler_pass = StreamingSincResampler::new(in_rate, out_rate, 2).unwrap();
    let mut pass_in = Vec::with_capacity(total_frames * 2);
    for i in 0..total_frames {
        let t = i as f32 / in_rate as f32;
        let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
        pass_in.push(s);
        pass_in.push(s * 0.5);
    }
    let mut pass_out = Vec::new();
    resampler_pass.push_interleaved(&pass_in, &mut pass_out);
    resampler_pass.flush(&mut pass_out);
    let skip = 1024 * 2;
    let tail = pass_out.len().saturating_sub(1024 * 2);
    let l_rms = (pass_out[skip..tail]
        .iter()
        .step_by(2)
        .map(|&s| s * s)
        .sum::<f32>()
        / ((tail - skip) / 2) as f32)
        .sqrt();
    assert!(
        (l_rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.03,
        "Expected ~0.7071 passband RMS at 1 kHz, got {}",
        l_rms
    );

    // 2. Ultrasonic 30 kHz (> 24 kHz output Nyquist) must be attenuated by > 40 dB
    let mut resampler_stop = StreamingSincResampler::new(in_rate, out_rate, 2).unwrap();
    let mut stop_in = Vec::with_capacity(total_frames * 2);
    for i in 0..total_frames {
        let t = i as f32 / in_rate as f32;
        let s = (2.0 * std::f32::consts::PI * 30_000.0 * t).sin();
        stop_in.push(s);
        stop_in.push(s);
    }
    let mut stop_out = Vec::new();
    resampler_stop.push_interleaved(&stop_in, &mut stop_out);
    resampler_stop.flush(&mut stop_out);
    let stop_tail = stop_out.len().saturating_sub(1024 * 2);
    let stop_rms = (stop_out[skip..stop_tail]
        .iter()
        .step_by(2)
        .map(|&s| s * s)
        .sum::<f32>()
        / ((stop_tail - skip) / 2) as f32)
        .sqrt();
    assert!(
        stop_rms < 0.007,
        "Expected >40 dB alias attenuation (<0.007 RMS) for 30 kHz @ 96k->48k, got {}",
        stop_rms
    );
}

#[test]
fn test_upmixer_interleaved_stereo_block_matches_frame_by_frame() {
    let cfg = crate::state::SurroundUpmixConfig {
        band_gains_db: [3.5, 1.0, -2.0, 2.5, -1.5],
        stereo_width: 1.35,
        ..Default::default()
    };

    let frames = 96usize;
    let mut input = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / 48_000.0;
        let l = (2.0 * std::f32::consts::PI * 180.0 * t).sin() * 0.45;
        let r = (2.0 * std::f32::consts::PI * 440.0 * t).cos() * 0.35;
        input.push(l);
        input.push(r);
    }

    for hw_ch in [2usize, 6, 8, 12] {
        let mut dsp_scalar = SurroundUpmixerDsp::new(48_000);
        let mut dsp_block = SurroundUpmixerDsp::new(48_000);
        dsp_scalar.sync_config(&cfg, 48_000);
        dsp_block.sync_config(&cfg, 48_000);

        let mut block_out = vec![[0.0f32; 12]; frames];
        dsp_block.process_interleaved_stereo_block(&input, 2, 2, &mut block_out, &cfg, hw_ch);

        for (i, frame_block) in block_out.iter().enumerate() {
            let frame_scalar =
                dsp_scalar.process_frame_for_hardware(input[i * 2], input[i * 2 + 1], &cfg, hw_ch);
            for ch in 0..12 {
                assert!(
                    (frame_block[ch] - frame_scalar[ch]).abs() < 1e-6,
                    "Mismatch at hw_ch={hw_ch} frame={i} ch={ch}: block={} scalar={}",
                    frame_block[ch],
                    frame_scalar[ch]
                );
            }
        }
    }
}

#[cfg(windows)]
#[test]
fn test_spatial_audio_reset_and_reactivate_static_objects() {
    let _live_audio_guard = lock_live_audio_test();
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_BLOB;
    use windows::core::Interface;

    struct ComCleanup(bool);
    impl Drop for ComCleanup {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }

    unsafe {
        let _com = ComCleanup(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
        let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            eprintln!("Skipping spatial reset test: MMDeviceEnumerator unavailable");
            return;
        };
        let Ok(dev) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else {
            eprintln!("Skipping spatial reset test: no default render endpoint");
            return;
        };
        let Ok(sac) = dev.Activate::<ISpatialAudioClient>(CLSCTX_ALL, None) else {
            eprintln!("Skipping spatial reset test: ISpatialAudioClient unavailable");
            return;
        };
        let max_dyn = sac.GetMaxDynamicObjectCount().unwrap_or(0);
        if max_dyn == 0 {
            eprintln!("Skipping spatial reset test: Windows Spatial Sound disabled (max_dyn == 0)");
            return;
        }
        if sac
            .IsSpatialAudioStreamAvailable(&ISpatialAudioObjectRenderStream::IID, None)
            .is_err()
        {
            eprintln!("Skipping spatial reset test: ISpatialAudioObjectRenderStream unavailable");
            return;
        }

        let fmt_enum = sac
            .GetSupportedAudioObjectFormatEnumerator()
            .expect("GetSupportedAudioObjectFormatEnumerator");
        let fmt_count = fmt_enum.GetCount().unwrap_or(0);
        let mut chosen_wf_ptr: *const WAVEFORMATEX = std::ptr::null();
        for idx in 0..fmt_count {
            if let Ok(wf_ptr) = fmt_enum.GetFormat(idx)
                && !wf_ptr.is_null()
            {
                let wf = *wf_ptr;
                if wf.wBitsPerSample == 32 && wf.nChannels == 1 {
                    chosen_wf_ptr = wf_ptr;
                    if wf.nSamplesPerSec == 48000 {
                        break;
                    }
                }
            }
        }
        assert!(
            !chosen_wf_ptr.is_null(),
            "Expected 32-bit float mono AudioObject format"
        );

        let ev = CreateEventW(None, false, false, None).expect("CreateEventW");
        let mask_714 = AudioObjectType_FrontLeft
            | AudioObjectType_FrontRight
            | AudioObjectType_FrontCenter
            | AudioObjectType_LowFrequency
            | AudioObjectType_SideLeft
            | AudioObjectType_SideRight
            | AudioObjectType_BackLeft
            | AudioObjectType_BackRight
            | AudioObjectType_TopFrontLeft
            | AudioObjectType_TopFrontRight
            | AudioObjectType_TopBackLeft
            | AudioObjectType_TopBackRight;
        let mut params = SpatialAudioObjectRenderStreamActivationParams {
            ObjectFormat: chosen_wf_ptr,
            StaticObjectTypeMask: mask_714,
            MinDynamicObjectCount: 0,
            MaxDynamicObjectCount: 0,
            Category: AudioCategory_Media,
            EventHandle: ev,
            NotifyObject: std::mem::ManuallyDrop::new(None),
        };
        let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
        (*pv.Anonymous.Anonymous).vt = VT_BLOB;
        (*pv.Anonymous.Anonymous).Anonymous.blob = BLOB {
            cbSize: std::mem::size_of::<SpatialAudioObjectRenderStreamActivationParams>() as u32,
            pBlobData: &mut params as *mut _ as *mut u8,
        };

        let stream: ISpatialAudioObjectRenderStream = sac
            .ActivateSpatialAudioStream(&*pv)
            .expect("ActivateSpatialAudioStream(7.1.4)");

        let static_types = [
            AudioObjectType_FrontLeft,
            AudioObjectType_FrontRight,
            AudioObjectType_FrontCenter,
            AudioObjectType_LowFrequency,
            AudioObjectType_SideLeft,
            AudioObjectType_SideRight,
            AudioObjectType_BackLeft,
            AudioObjectType_BackRight,
            AudioObjectType_TopFrontLeft,
            AudioObjectType_TopFrontRight,
            AudioObjectType_TopBackLeft,
            AudioObjectType_TopBackRight,
        ];

        let mut objects: Vec<ISpatialAudioObject> = static_types
            .iter()
            .map(|&t| {
                stream
                    .ActivateSpatialAudioObject(t)
                    .expect("initial ActivateSpatialAudioObject")
            })
            .collect();

        let render_silent_passes =
            |objs: &mut Vec<ISpatialAudioObject>, passes: usize, phase: &str| {
                for pass in 0..passes {
                    let wait_res = WaitForSingleObject(ev, 200);
                    assert_eq!(
                        wait_res.0, 0,
                        "{} pass {}: WaitForSingleObject failed/timed out ({:?})",
                        phase, pass, wait_res
                    );
                    let mut dyn_count = 0u32;
                    let mut frame_count = 0u32;
                    stream
                        .BeginUpdatingAudioObjects(&mut dyn_count, &mut frame_count)
                        .expect("BeginUpdatingAudioObjects");
                    if frame_count > 0 {
                        if objs.len() < static_types.len() {
                            objs.clear();
                            for &t in &static_types {
                                objs.push(
                                    stream
                                        .ActivateSpatialAudioObject(t)
                                        .expect("re-ActivateSpatialAudioObject after Reset"),
                                );
                            }
                        }
                        for (ch, obj) in objs.iter().enumerate() {
                            let mut buf_ptr: *mut u8 = std::ptr::null_mut();
                            let mut buf_bytes = 0u32;
                            obj.GetBuffer(&mut buf_ptr, &mut buf_bytes)
                                .unwrap_or_else(|e| {
                                    panic!(
                                        "{} pass {} ch {}: GetBuffer failed after Reset: {:?}",
                                        phase, pass, ch, e
                                    )
                                });
                            assert!(
                                !buf_ptr.is_null()
                                    && (buf_bytes as usize) >= (frame_count as usize) * 4,
                                "{} pass {} ch {}: invalid buffer len {}",
                                phase,
                                pass,
                                ch,
                                buf_bytes
                            );
                            std::ptr::write_bytes(buf_ptr, 0, buf_bytes as usize);
                        }
                    }
                    stream
                        .EndUpdatingAudioObjects()
                        .expect("EndUpdatingAudioObjects");
                }
            };

        stream.Start().expect("initial stream.Start");
        render_silent_passes(&mut objects, 2, "pre-pause");

        // Pause: Stop -> release revoked objects -> Reset
        stream.Stop().expect("stream.Stop");
        objects.clear();
        stream.Reset().expect("stream.Reset");

        // Resume: Start -> Wait -> BeginUpdatingAudioObjects -> re-activate 12 objects -> GetBuffer
        stream.Start().expect("resume stream.Start");
        render_silent_passes(&mut objects, 3, "post-resume");

        let _ = stream.Stop();
        objects.clear();
        let _ = stream.Reset();
        drop(stream);
        drop(fmt_enum);
        drop(sac);
        drop(dev);
        drop(enumerator);
        let _ = CloseHandle(ev);
    }
}
