#[cfg(target_os = "android")]
pub use rt_core::android_log;
pub use rt_core::{biquad, lyrics, playlist, state, touch};

pub use rt_decode::{android_video, iso, mpegh, openmpt_ffi};

pub use rt_audio::{audio, bitstream, devices};

pub use rt_render::engine;

#[cfg(not(target_os = "android"))]
pub use rt_tui::ui;

#[cfg(target_os = "android")]
pub mod android;
pub mod ipc;
