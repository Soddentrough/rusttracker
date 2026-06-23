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
