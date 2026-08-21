#[cfg(target_os = "android")]
pub mod decoder {
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use crossbeam_channel::{bounded, unbounded};
    use crate::state::{AppState, VideoFrame};

    #[repr(C)]
    struct AMediaExtractor { _private: [u8; 0] }
    #[repr(C)]
    struct AMediaCodec { _private: [u8; 0] }
    #[repr(C)]
    struct AMediaFormat { _private: [u8; 0] }

    #[repr(C)]
    struct AMediaCodecBufferInfo {
        offset: i32,
        size: i32,
        presentation_time_us: i64,
        flags: u32,
    }

    const AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC: i32 = 0;
    const AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED: isize = -2;
    const AMEDIACODEC_INFO_TRY_AGAIN_LATER: isize = -1;
    const AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM: u32 = 4;

    #[link(name = "mediandk")]
    unsafe extern "C" {
        fn AMediaExtractor_new() -> *mut AMediaExtractor;
        fn AMediaExtractor_delete(extractor: *mut AMediaExtractor) -> i32;
        fn AMediaExtractor_setDataSourceFd(extractor: *mut AMediaExtractor, fd: i32, offset: i64, length: i64) -> i32;
        fn AMediaExtractor_getTrackCount(extractor: *mut AMediaExtractor) -> usize;
        fn AMediaExtractor_getTrackFormat(extractor: *mut AMediaExtractor, idx: usize) -> *mut AMediaFormat;
        fn AMediaExtractor_selectTrack(extractor: *mut AMediaExtractor, idx: usize) -> i32;
        fn AMediaExtractor_seekTo(extractor: *mut AMediaExtractor, seekPosUs: i64, mode: i32) -> i32;
        fn AMediaExtractor_advance(extractor: *mut AMediaExtractor) -> bool;
        fn AMediaExtractor_readSampleData(extractor: *mut AMediaExtractor, buffer: *mut u8, capacity: usize) -> isize;
        fn AMediaExtractor_getSampleTime(extractor: *mut AMediaExtractor) -> i64;
        fn AMediaExtractor_getSampleFlags(extractor: *mut AMediaExtractor) -> u32;

        fn AMediaFormat_delete(format: *mut AMediaFormat) -> i32;
        fn AMediaFormat_getString(format: *mut AMediaFormat, name: *const c_char, out: *mut *const c_char) -> bool;
        fn AMediaFormat_getInt32(format: *mut AMediaFormat, name: *const c_char, out: *mut i32) -> bool;

        fn AMediaCodec_createDecoderByType(mime_type: *const c_char) -> *mut AMediaCodec;
        fn AMediaCodec_configure(codec: *mut AMediaCodec, format: *const AMediaFormat, surface: *mut c_void, crypto: *mut c_void, flags: u32) -> i32;
        fn AMediaCodec_start(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_stop(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_delete(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_flush(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_dequeueInputBuffer(codec: *mut AMediaCodec, timeoutUs: i64) -> isize;
        fn AMediaCodec_getInputBuffer(codec: *mut AMediaCodec, idx: usize, out_size: *mut usize) -> *mut u8;
        fn AMediaCodec_queueInputBuffer(codec: *mut AMediaCodec, idx: usize, offset: usize, size: usize, pts: u64, flags: u32) -> i32;
        fn AMediaCodec_dequeueOutputBuffer(codec: *mut AMediaCodec, info: *mut AMediaCodecBufferInfo, timeoutUs: i64) -> isize;
        fn AMediaCodec_getOutputBuffer(codec: *mut AMediaCodec, idx: usize, out_size: *mut usize) -> *mut u8;
        fn AMediaCodec_getOutputFormat(codec: *mut AMediaCodec) -> *mut AMediaFormat;
        fn AMediaCodec_releaseOutputBuffer(codec: *mut AMediaCodec, idx: usize, render: bool) -> i32;
    }

    pub fn start_android_video_thread(
        path: &str,
        shared_state: Arc<Mutex<AppState>>,
        stop_token: Arc<AtomicBool>,
    ) -> bool {
        let path_string = path.to_string();

        let (video_frame_tx, video_frame_rx) = bounded::<VideoFrame>(16);
        let (free_video_frame_tx, free_video_frame_rx) = unbounded::<VideoFrame>();

        for _ in 0..16 {
            let _ = free_video_frame_tx.try_send(VideoFrame {
                pts: 0.0,
                width: 0,
                height: 0,
                y_plane: Vec::new(),
                u_plane: Vec::new(),
                v_plane: Vec::new(),
                y_stride: 0,
                u_stride: 0,
                v_stride: 0,
                bit_depth: 8,
                color_space: 0,
                color_range: 0,
                color_trc: 0,
            });
        }

        if let Ok(mut state) = shared_state.lock() {
            state.video_frame_rx = Some(video_frame_rx);
            state.free_video_frame_tx = Some(free_video_frame_tx.clone());
        }

        let state_for_video = shared_state.clone();
        let stop_token_for_video = stop_token.clone();

        std::thread::Builder::new()
            .name("AndroidVideoDecoder".to_string())
            .spawn(move || unsafe {
                let extractor = AMediaExtractor_new();
                if extractor.is_null() {
                    eprintln!("[RustTracker Video] Failed to create AMediaExtractor");
                    return;
                }

                use std::os::fd::{FromRawFd, IntoRawFd};
                let Ok(file) = std::fs::File::open(&path_string) else {
                    eprintln!("[RustTracker Video] Failed to open local file {}", path_string);
                    AMediaExtractor_delete(extractor);
                    return;
                };
                let file_len = file.metadata().map(|m| m.len() as i64).unwrap_or(0);
                let raw_fd = file.into_raw_fd();
                let status = AMediaExtractor_setDataSourceFd(extractor, raw_fd, 0, file_len);
                if status != 0 {
                    eprintln!("[RustTracker Video] Failed to set data source FD on AMediaExtractor (status {}) for {}", status, path_string);
                    let _ = std::fs::File::from_raw_fd(raw_fd);
                    AMediaExtractor_delete(extractor);
                    return;
                }

                let track_count = AMediaExtractor_getTrackCount(extractor);
                let mut video_track_idx = None;
                let mut codec = ptr::null_mut();
                let mut vid_w = 1920i32;
                let mut vid_h = 1080i32;

                let mime_key = CStr::from_bytes_with_nul(b"mime\0").unwrap();
                let width_key = CStr::from_bytes_with_nul(b"width\0").unwrap();
                let height_key = CStr::from_bytes_with_nul(b"height\0").unwrap();
                let stride_key = CStr::from_bytes_with_nul(b"stride\0").unwrap();
                let slice_height_key = CStr::from_bytes_with_nul(b"slice-height\0").unwrap();

                for i in 0..track_count {
                    let format = AMediaExtractor_getTrackFormat(extractor, i);
                    if format.is_null() { continue; }

                    let mut mime_ptr: *const c_char = ptr::null();
                    if AMediaFormat_getString(format, mime_key.as_ptr(), &mut mime_ptr) && !mime_ptr.is_null() {
                        let mime_cstr = CStr::from_ptr(mime_ptr);
                        if let Ok(mime_str) = mime_cstr.to_str() {
                            if mime_str.starts_with("video/") {
                                video_track_idx = Some(i);
                                AMediaFormat_getInt32(format, width_key.as_ptr(), &mut vid_w);
                                AMediaFormat_getInt32(format, height_key.as_ptr(), &mut vid_h);

                                codec = AMediaCodec_createDecoderByType(mime_ptr);
                                if !codec.is_null() {
                                    let status = AMediaCodec_configure(codec, format, ptr::null_mut(), ptr::null_mut(), 0);
                                    if status == 0 && AMediaCodec_start(codec) == 0 {
                                        AMediaExtractor_selectTrack(extractor, i);
                                        AMediaFormat_delete(format);
                                        eprintln!("[RustTracker Video] Hardware decoder started for track {} ({} {}x{})", i, mime_str, vid_w, vid_h);
                                        break;
                                    } else {
                                        AMediaCodec_delete(codec);
                                        codec = ptr::null_mut();
                                    }
                                }
                            }
                        }
                    }
                    AMediaFormat_delete(format);
                }

                if codec.is_null() || video_track_idx.is_none() {
                    eprintln!("[RustTracker Video] No suitable hardware video decoder found for {}", path_string);
                    AMediaExtractor_delete(extractor);
                    let _ = std::fs::File::from_raw_fd(raw_fd);
                    return;
                }

                if let Ok(mut state) = state_for_video.lock() {
                    state.has_video_stream = true;
                    state.video_info = Some(format!("Video Stream: {}x{} H.264 (Hardware)", vid_w, vid_h));
                }

                let mut local_epoch = 0;
                let mut eos_input = false;
                let mut out_stride = vid_w as usize;
                let mut out_slice_h = vid_h as usize;

                while !stop_token_for_video.load(Ordering::Relaxed) {
                    // Check seek
                    {
                        if let Ok(state) = state_for_video.lock() {
                            if state.seek_epoch > local_epoch {
                                let seek_us = (state.current_seconds * 1_000_000.0) as i64;
                                AMediaExtractor_seekTo(extractor, seek_us, AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC);
                                AMediaCodec_flush(codec);
                                local_epoch = state.seek_epoch;
                                eos_input = false;
                            }
                        }
                    }

                    // Feed input buffer
                    if !eos_input {
                        let in_idx = AMediaCodec_dequeueInputBuffer(codec, 2000);
                        if in_idx >= 0 {
                            let mut buf_cap: usize = 0;
                            let in_buf = AMediaCodec_getInputBuffer(codec, in_idx as usize, &mut buf_cap);
                            if !in_buf.is_null() && buf_cap > 0 {
                                let sample_size = AMediaExtractor_readSampleData(extractor, in_buf, buf_cap);
                                if sample_size > 0 {
                                    let sample_pts = AMediaExtractor_getSampleTime(extractor);
                                    let sample_flags = AMediaExtractor_getSampleFlags(extractor);
                                    AMediaCodec_queueInputBuffer(codec, in_idx as usize, 0, sample_size as usize, sample_pts as u64, sample_flags);
                                    AMediaExtractor_advance(extractor);
                                } else {
                                    AMediaCodec_queueInputBuffer(codec, in_idx as usize, 0, 0, 0, AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM);
                                    eos_input = true;
                                }
                            }
                        }
                    }

                    // Drain output buffer
                    let mut info = AMediaCodecBufferInfo { offset: 0, size: 0, presentation_time_us: 0, flags: 0 };
                    let out_idx = AMediaCodec_dequeueOutputBuffer(codec, &mut info, 5000);

                    if out_idx >= 0 {
                        let pts_sec = info.presentation_time_us as f64 / 1_000_000.0;
                        
                        // Audio-video sync: check current audio timestamp
                        let current_audio_pts = state_for_video.lock().map(|s| s.current_seconds).unwrap_or(pts_sec);
                        if pts_sec > current_audio_pts + 0.08 {
                            let wait_ms = ((pts_sec - current_audio_pts) * 1000.0).min(50.0) as u64;
                            if wait_ms > 0 {
                                std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                            }
                        }

                        let mut out_size: usize = 0;
                        let out_buf = AMediaCodec_getOutputBuffer(codec, out_idx as usize, &mut out_size);
                        let offset = info.offset as usize;
                        if !out_buf.is_null() && out_size > offset {
                            let available_len = out_size - offset;
                            let src_slice = std::slice::from_raw_parts(out_buf.add(offset), available_len);

                            let mut frame = match free_video_frame_rx.try_recv() {
                                Ok(f) => f,
                                Err(_) => VideoFrame {
                                    pts: pts_sec,
                                    width: vid_w as u32,
                                    height: vid_h as u32,
                                    y_plane: Vec::new(),
                                    u_plane: Vec::new(),
                                    v_plane: Vec::new(),
                                    y_stride: vid_w as usize,
                                    u_stride: (vid_w / 2) as usize,
                                    v_stride: (vid_w / 2) as usize,
                                    bit_depth: 8,
                                    color_space: 0,
                                    color_range: 0,
                                    color_trc: 0,
                                },
                            };

                            let w = vid_w as usize;
                            let h = vid_h as usize;
                            let stride = out_stride.max(w);
                            let slice_h = out_slice_h.max(h);
                            let half_w = w / 2;
                            let half_h = h / 2;

                            frame.pts = pts_sec;
                            frame.width = vid_w as u32;
                            frame.height = vid_h as u32;
                            frame.y_stride = w;
                            frame.u_stride = half_w;
                            frame.v_stride = half_w;
                            frame.bit_depth = 8;
                            frame.color_space = 0;
                            frame.color_range = 0;
                            frame.color_trc = 0;

                            frame.y_plane.resize(w * h, 0);
                            frame.u_plane.resize(half_w * half_h, 128);
                            frame.v_plane.resize(half_w * half_h, 128);

                            // Copy Y plane row by row
                            for row in 0..h {
                                let src_offset = row * stride;
                                let dst_offset = row * w;
                                if src_offset + w <= src_slice.len() {
                                    frame.y_plane[dst_offset..dst_offset + w].copy_from_slice(&src_slice[src_offset..src_offset + w]);
                                }
                            }

                            // Semi-planar (NV12) UV plane
                            let uv_start = stride * slice_h;
                            if uv_start < src_slice.len() {
                                for row in 0..half_h {
                                    let src_row = uv_start + row * stride;
                                    let dst_row = row * half_w;
                                    for col in 0..half_w {
                                        let src_idx = src_row + col * 2;
                                        if src_idx + 1 < src_slice.len() {
                                            frame.u_plane[dst_row + col] = src_slice[src_idx];
                                            frame.v_plane[dst_row + col] = src_slice[src_idx + 1];
                                        }
                                    }
                                }
                            }

                            let _ = video_frame_tx.try_send(frame);
                        }

                        AMediaCodec_releaseOutputBuffer(codec, out_idx as usize, false);
                    } else if out_idx == AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED {
                        let out_format = AMediaCodec_getOutputFormat(codec);
                        if !out_format.is_null() {
                            let mut new_w = vid_w;
                            let mut new_h = vid_h;
                            let mut new_stride = vid_w;
                            let mut new_slice_h = vid_h;
                            AMediaFormat_getInt32(out_format, width_key.as_ptr(), &mut new_w);
                            AMediaFormat_getInt32(out_format, height_key.as_ptr(), &mut new_h);
                            AMediaFormat_getInt32(out_format, stride_key.as_ptr(), &mut new_stride);
                            AMediaFormat_getInt32(out_format, slice_height_key.as_ptr(), &mut new_slice_h);
                            
                            vid_w = new_w;
                            vid_h = new_h;
                            out_stride = if new_stride > 0 { new_stride as usize } else { new_w as usize };
                            out_slice_h = if new_slice_h > 0 { new_slice_h as usize } else { new_h as usize };
                            eprintln!("[RustTracker Video] Output format changed: {}x{}, stride={}, slice_h={}", vid_w, vid_h, out_stride, out_slice_h);
                            AMediaFormat_delete(out_format);
                        }
                    } else if out_idx == AMEDIACODEC_INFO_TRY_AGAIN_LATER {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                }

                AMediaCodec_stop(codec);
                AMediaCodec_delete(codec);
                AMediaExtractor_delete(extractor);
                let _ = std::fs::File::from_raw_fd(raw_fd);
                eprintln!("[RustTracker Video] Decoder cleanly shut down");
            })
            .is_ok()
    }
}
