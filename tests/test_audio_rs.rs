#[path = "../src/bitstream.rs"]
pub mod bitstream;
#[path = "../src/audio.rs"]
pub mod audio;
#[path = "../src/state.rs"]
pub mod state;

#[test]
fn test_midi_loading() {
    let result = audio::load_audio_source("audio_tests/darude-sandstorm.mid");
    assert!(result.is_ok(), "Failed to load MIDI: {:?}", result.err());
    let mut source = result.unwrap();
    println!("Parsed MIDI duration: {}s", source.get_duration_seconds());
}

#[test]
fn test_audio_transition_and_fallback() {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new("Test App".to_string())));
    
    println!("Starting first audio track (MOD)...");
    let handle1 = audio::start_audio_thread("audio_tests/ive_got_the_power.mod", false, shared_state.clone());
    assert!(handle1.is_ok(), "Failed to start first audio thread: {:?}", handle1.err());
    let handle1 = handle1.unwrap();
    
    std::thread::sleep(std::time::Duration::from_secs(1));
    
    println!("Stopping first audio track (dropping handle)...");
    drop(handle1);
    
    std::thread::sleep(std::time::Duration::from_millis(200));
    
    println!("Starting second audio track (MIDI transition)...");
    let handle2 = audio::start_audio_thread("audio_tests/darude-sandstorm.mid", false, shared_state.clone());
    assert!(handle2.is_ok(), "Failed to start second audio track: {:?}", handle2.err());
    let handle2 = handle2.unwrap();
    
    std::thread::sleep(std::time::Duration::from_secs(1));
    
    println!("Stopping second audio track (dropping handle)...");
    drop(handle2);
    
    println!("Audio transition test completed successfully!");
}

#[test]
fn test_video_transition_and_fallback() {
    let shared_state = std::sync::Arc::new(std::sync::Mutex::new(state::AppState::new("Test App".to_string())));
    let movie_path = "/home/naoki/Music/Jeff Wayne's Musical Version Of The War Of The Worlds The New Generation (2013) [1080p] [BluRay] [5.1] [YTS.MX]/Jeff.Wayne's.Musical.Version.Of.The.War.Of.The.Worlds.The.New.Generation.2013.1080p.BluRay.x264.AAC5.1-[YTS.MX].mp4";
    
    if std::path::Path::new(movie_path).exists() {
        println!("Starting video audio track...");
        let handle = audio::start_audio_thread(movie_path, false, shared_state.clone());
        assert!(handle.is_ok(), "Failed to start video audio thread: {:?}", handle.err());
        let handle = handle.unwrap();
        
        std::thread::sleep(std::time::Duration::from_secs(1));
        
        println!("Stopping video audio track...");
        drop(handle);
    } else {
        println!("Movie file not found, skipping video transition test.");
    }
}
