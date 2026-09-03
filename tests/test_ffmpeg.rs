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
fn test_aac_5_1_downmix() {
    let file_path = "audio_tests/AAC 5.1.mp4";
    let mut source = rusttracker::audio::load_audio_source(file_path).expect("Failed to load AAC 5.1.mp4");
    assert_eq!(source.get_num_channels(), 6);

    let mut stereo_buf = vec![0.0f32; 1024 * 2];
    let frames = source.read_frames(2, 48000, &mut stereo_buf);
    assert!(frames > 0, "Expected frames read > 0");
    assert!(stereo_buf.iter().any(|&s| s.abs() > 0.0), "Expected non-silent downmixed stereo audio");
}

