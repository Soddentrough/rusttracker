pub use rt_core::{lyrics, playlist, state, touch};

#[doc(hidden)]
pub mod audio {
    pub use rt_core::devices::SYSTEM_DEFAULT_AUDIO_DEVICE;
    pub use rt_core::state::{push_planar_lookahead_slices, update_lookahead_timeline_from_queue};
}

pub mod engine;
