use std::fs::File;
use std::io::Cursor;

fn build_wav_bytes(sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
    let bits_per_sample = 16u16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * block_align as u32;
    let data_size = (frames * block_align as usize) as u32;
    let riff_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());

    for f in 0..frames {
        let t = f as f32 / sample_rate as f32;
        for c in 0..channels {
            let freq = 220.0 * (1.0 + c as f32 * 0.5);
            let sample = ((t * freq * std::f32::consts::TAU).sin() * 16000.0) as i16;
            buf.extend_from_slice(&sample.to_le_bytes());
        }
    }
    buf
}

#[test]
fn test_symphonia_synthesized_multichannel_wav() {
    for &channels in &[1u16, 2, 6, 8] {
        let wav_bytes = build_wav_bytes(48_000, channels, 4800);
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
    let file_path = "audio_tests/FLAC 5.1.flac";
    if !std::path::Path::new(file_path).exists() {
        return;
    }
    let file = File::open(file_path).expect("Failed to open FLAC 5.1.flac");
    let mut source = rusttracker::audio::try_symphonia(file, "flac", "flac", None, Some(file_path))
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
        "Expected non-zero audio energy from 5.1 FLAC, got {}",
        total_energy
    );
}
