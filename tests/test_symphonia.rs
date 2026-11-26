mod common;

use std::fs::File;
use std::io::Cursor;

#[test]
fn test_symphonia_synthesized_multichannel_wav() {
    for &channels in &[1u16, 2, 6, 8] {
        let wav_bytes = common::build_wav_bytes(48_000, channels, 4800);
        let cursor = Cursor::new(wav_bytes);
        let mut source = rusttracker::audio::try_symphonia(cursor, "wav", "wav", None, None)
            .unwrap_or_else(|e| panic!("Symphonia failed on {}-ch WAV: {:?}", channels, e));

        assert_eq!(source.get_num_channels(), channels as i32);
        assert_eq!(source.get_intrinsic_sample_rate(), Some(48_000));

        let mut stereo_out = vec![0.0f32; 1024 * 2];
        let frames_read = source.read_frames(2, 48_000, &mut stereo_out);
        assert!(
            frames_read > 0,
            "Expected frames read > 0 for {}-ch WAV",
            channels
        );
        let rms = (stereo_out[..frames_read * 2]
            .iter()
            .map(|&s| s * s)
            .sum::<f32>()
            / (frames_read * 2) as f32)
            .sqrt();
        assert!(
            rms > 0.05,
            "Expected active audio RMS > 0.05 for {}-ch WAV downmixed to stereo, got {}",
            channels,
            rms
        );
    }
}

#[test]
fn test_symphonia_flac_5_1() {
    let tmp_dir = common::TempTestDir::new("symphonia_flac_51");
    let synth_path = tmp_dir.path().join("synthetic_5_1.flac");
    common::encode_synthetic_flac(&synth_path, 6, 48_000, 1.5).expect("encode synthetic 5.1 FLAC");

    let mut targets = vec![synth_path];
    let local_fixture = std::path::PathBuf::from("audio_tests/FLAC 5.1.flac");
    if local_fixture.exists() {
        targets.push(local_fixture);
    }

    for path in targets {
        let file = File::open(&path).expect("Failed to open 5.1 FLAC");
        let path_str = path.to_string_lossy();
        let mut source =
            rusttracker::audio::try_symphonia(file, "flac", "flac", None, Some(&path_str))
                .expect("Symphonia must decode 5.1 channel FLAC");

        assert_eq!(source.get_num_channels(), 6);
        assert!(source.get_duration_seconds() > 1.0);

        let mut stereo_buf = vec![0.0f32; 2048 * 2];
        let mut total_energy = 0.0f32;
        for _ in 0..10 {
            let n = source.read_frames(2, 48_000, &mut stereo_buf);
            assert!(n > 0, "Expected decoded frames from 5.1 FLAC");
            total_energy += stereo_buf[..n * 2].iter().map(|&s| s * s).sum::<f32>();
        }
        assert!(
            total_energy > 0.01,
            "Expected non-zero audio energy from 5.1 FLAC ({}), got {}",
            path.display(),
            total_energy
        );
    }
}
