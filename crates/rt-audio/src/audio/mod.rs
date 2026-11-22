use anyhow::Result;
use crossbeam_channel::Receiver;
use std::sync::Arc;

pub use rt_core::state::{push_planar_lookahead_slices, update_lookahead_timeline_from_queue};

mod dsp;
mod playback;
mod sources;
mod topology;
mod upmixer;
mod video_decode;

pub use dsp::*;
pub use playback::*;
pub use sources::*;
pub use topology::*;
pub use upmixer::*;
pub use video_decode::*;

pub(in crate::audio) use dsp::{dsp_recycle_channel, spawn_dsp_thread_with_recycle};
pub(in crate::audio) use playback::{
    decode_single_audio_chunk, process_decoder_state_requests, push_chunk_lookahead_slices,
};
pub(in crate::audio) use video_decode::spawn_media_decoder_threads;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct CpalStreamKeepAlive {
    _audio_shutdown_tx: Option<crossbeam_channel::Sender<()>>,
    _dsp_rx_guard: Option<Receiver<DspMessage>>,
    _dsp_recycle_rx_guard: Option<Receiver<DspMessage>>,
}

pub enum PlaybackHandle {
    // Field order is load-bearing: `cpal::Stream` MUST be declared before `CpalStreamKeepAlive`
    // so Rust drops and joins the OS real-time audio callback thread before dropping the channel
    // receivers, guaranteeing `try_send` in the RT callback never hits `Disconnected` and drops
    // heap-backed `AudioChunk` / `DspMessage` buffers on the RT thread.
    Cpal(
        cpal::Stream,
        Arc<std::sync::atomic::AtomicBool>,
        CpalStreamKeepAlive,
    ),
    Bitstream(
        Option<std::thread::JoinHandle<()>>,
        Arc<std::sync::atomic::AtomicBool>,
    ),
    Dummy(
        Option<std::thread::JoinHandle<()>>,
        Arc<std::sync::atomic::AtomicBool>,
    ),
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        match self {
            PlaybackHandle::Cpal(stream, stop_token, _keep_alive) => {
                let _ = &stream;
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            PlaybackHandle::Bitstream(handle_opt, stop_token) => {
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(handle) = handle_opt.take() {
                    let _ = handle.join();
                }
            }
            PlaybackHandle::Dummy(handle_opt, stop_token) => {
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(handle) = handle_opt.take() {
                    let _ = handle.join();
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct DspMessage {
    pub audio_data: Vec<f32>,
    pub channel_vus: Vec<f32>,
    pub current_order: i32,
    pub current_row: i32,
    pub bpm: i32,
    pub speed: i32,
    pub current_seconds: f64,
    pub current_row_string: String,
    pub channel_audio_data: Vec<Vec<f32>>,
    pub seek_epoch: u64,
}

pub trait AudioSource: Send {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize;
    fn get_duration_seconds(&mut self) -> f64;
    fn get_position_seconds(&mut self) -> f64;
    fn set_position_seconds(&mut self, pos: f64);
    fn get_num_channels(&mut self) -> i32;
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32;
    fn get_artist(&mut self) -> String;
    fn get_type(&mut self) -> String;
    fn get_tempo(&mut self) -> i32;
    fn get_speed(&mut self) -> i32;
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32>;
    fn get_num_samples(&mut self) -> i32;
    fn get_num_instruments(&mut self) -> i32;
    fn get_num_patterns(&mut self) -> i32;
    fn get_current_order(&mut self) -> i32;
    fn get_current_row(&mut self) -> i32;
    fn get_tracker_channels(&mut self) -> Option<i32> {
        None
    }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn get_current_row_string(&mut self) -> String {
        String::new()
    }
    fn get_video_info(&mut self) -> Option<String> {
        None
    }
    fn attach_video_queue(&mut self, _tx: crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>) {}
    fn take_video_parameters(
        &mut self,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational, u32)> {
        None
    }
    fn get_bitrate(&mut self) -> Option<u32> {
        None
    }
    fn get_video_fps(&self) -> f32 {
        0.0
    }
    fn get_video_bitrate(&self) -> Option<u32> {
        None
    }
    fn get_audio_tracks(&self) -> Vec<crate::state::AudioTrackInfo> {
        Vec::new()
    }
    fn get_selected_audio_track(&self) -> usize {
        0
    }
    fn select_audio_track(&mut self, _track_idx: usize) -> Result<()> {
        Ok(())
    }
    fn get_active_audio_tracks(&self) -> Vec<usize> {
        vec![self.get_selected_audio_track()]
    }
    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if let Some(&(idx, _)) = tracks.first() {
            self.select_audio_track(idx)
        } else {
            Ok(())
        }
    }
    fn has_video_stream(&self) -> bool {
        false
    }
    fn is_native_atmos(&self) -> bool {
        false
    }
}

pub struct AudioChunk {
    pub(super) samples: Vec<f32>,
    pub(super) valid_frames: usize,
    pub(super) channel_vus: Vec<f32>,
    pub(super) left_peak: f32,
    pub(super) right_peak: f32,
    pub(super) current_order: i32,
    pub(super) current_row: i32,
    pub(super) bpm: i32,
    pub(super) speed: i32,
    pub(super) current_seconds: f64,
    pub(super) current_row_string: String,
    pub(super) tracker_channels: Option<i32>,
    pub(super) spatial_channels: i32,
    pub(super) track_ended: bool,
    pub(super) seek_epoch: u64,
}
