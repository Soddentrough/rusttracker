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

