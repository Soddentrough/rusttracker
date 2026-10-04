pub mod audio;
pub mod bitstream;
pub mod engine;
pub mod ipc;
pub mod lyrics;
pub mod playlist;
pub mod state;
pub mod touch;
#[cfg(not(target_os = "android"))]
pub mod ui;

#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_os = "android")]
pub mod android_video;
