use std::env;

fn main() {
    ffmpeg_next::init().unwrap();
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: test_ffmpeg_seek <file>");
        return;
    }
    let file = &args[1];
    println!("Opening: {}", file);
    
    let mut ictx = ffmpeg_next::format::input(&file).unwrap();
    
    let mut video_stream_idx = None;
    let mut audio_stream_idx = None;
    
    for stream in ictx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Video {
            video_stream_idx = Some(stream.index());
        } else if stream.parameters().medium() == ffmpeg_next::media::Type::Audio {
            audio_stream_idx = Some(stream.index());
        }
    }
    
    println!("Video stream: {:?}, Audio stream: {:?}", video_stream_idx, audio_stream_idx);
    
    // Seek to 30 seconds
    let pos = 30.0;
    
    // Test 1: Global Seek
    let pts1 = (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;
    let ret1 = unsafe {
        ffmpeg_next::ffi::av_seek_frame(ictx.as_mut_ptr(), -1, pts1, ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD)
    };
    println!("Global Seek (-1) return code: {}", ret1);
    
    // Test 2: Video Stream Seek
    if let Some(idx) = video_stream_idx {
        let stream = ictx.stream(idx).unwrap();
        let tb = stream.time_base();
        let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;
        let pts2 = (pos / tb_f64) as i64;
        let ret2 = unsafe {
            ffmpeg_next::ffi::av_seek_frame(ictx.as_mut_ptr(), idx as i32, pts2, ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD)
        };
        println!("Video Seek ({}) return code: {}", idx, ret2);
    }
    
    // Test 3: Audio Stream Seek
    if let Some(idx) = audio_stream_idx {
        let stream = ictx.stream(idx).unwrap();
        let tb = stream.time_base();
        let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;
        let pts3 = (pos / tb_f64) as i64;
        let ret3 = unsafe {
            ffmpeg_next::ffi::av_seek_frame(ictx.as_mut_ptr(), idx as i32, pts3, ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD)
        };
        println!("Audio Seek ({}) return code: {}", idx, ret3);
    }
}
