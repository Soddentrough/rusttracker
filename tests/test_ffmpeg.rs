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
    if !std::path::Path::new(file_path).exists() {
        return;
    }
    let mut source = rusttracker::audio::load_audio_source(file_path).expect("Failed to load AAC 5.1.mp4");
    assert_eq!(source.get_num_channels(), 6);

    let mut stereo_buf = vec![0.0f32; 1024 * 2];
    let frames = source.read_frames(2, 48000, &mut stereo_buf);
    assert!(frames > 0, "Expected frames read > 0");
    assert!(stereo_buf.iter().any(|&s| s.abs() > 0.0), "Expected non-silent downmixed stereo audio");
}

#[test]
fn test_video_rotation_90() {
    for file_path in ["audio_tests/portrait_test_90.mp4", "audio_tests/portrait_test_90.mkv"] {
        if !std::path::Path::new(file_path).exists() {
            continue;
        }
        let mut source = rusttracker::audio::load_audio_source(file_path)
            .unwrap_or_else(|e| panic!("Failed to load {}: {}", file_path, e));
        
        // Video info should show rotated dimensions (360x640 instead of 640x360)
        let info = source.get_video_info().expect("Expected video info");
        assert!(info.contains("360x640"), "Expected rotated dimensions 360x640 for {}, got: {}", file_path, info);

        // take_video_parameters should report rotation 90
        let (_params, _tb, rotation) = source.take_video_parameters().expect("Expected video parameters");
        assert_eq!(rotation, 90, "Expected rotation == 90 degrees for {}", file_path);
    }
}


