use super::*;
use crate::state::AppState;
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct VideoStreamSetup {
    pub stream_index: usize,
    pub params: ffmpeg_next::codec::Parameters,
    pub time_base: ffmpeg_next::Rational,
    pub rotation: u32,
    pub info: String,
    pub fps: f32,
    pub bitrate_kbps: Option<u32>,
}

pub fn probe_stream_bitrate_kbps(stream: &ffmpeg_next::format::stream::Stream) -> Option<u32> {
    let raw_br = unsafe { (*stream.parameters().as_ptr()).bit_rate };
    if raw_br > 0 {
        return Some((raw_br / 1000).max(1) as u32);
    }
    let meta = stream.metadata();
    for (k, v) in meta.iter() {
        let ku = k.to_uppercase();
        if (ku == "BPS" || ku.starts_with("BPS-") || ku == "BIT_RATE" || ku == "VARIANT_BITRATE")
            && let Ok(bps) = v.trim().parse::<u64>()
            && bps > 0
        {
            return Some((bps / 1000).max(1) as u32);
        }
    }
    None
}

pub fn probe_video_stream(ictx: &ffmpeg_next::format::context::Input) -> Option<VideoStreamSetup> {
    let v_stream = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .filter(|s| {
            !s.disposition()
                .contains(ffmpeg_next::format::stream::Disposition::ATTACHED_PIC)
        })
        .or_else(|| {
            ictx.streams().find(|s| {
                s.parameters().medium() == ffmpeg_next::media::Type::Video
                    && !s
                        .disposition()
                        .contains(ffmpeg_next::format::stream::Disposition::ATTACHED_PIC)
            })
        })?;
    let stream_index = v_stream.index();
    let params = v_stream.parameters();
    let time_base = v_stream.time_base();
    let mut rotation = 0u32;

    let avg_fr = v_stream.avg_frame_rate();
    let r_fr = v_stream.rate();
    let fps = if avg_fr.denominator() > 0 && avg_fr.numerator() > 0 {
        let f = avg_fr.numerator() as f32 / avg_fr.denominator() as f32;
        if (1.0..=1000.0).contains(&f) {
            f
        } else if r_fr.denominator() > 0 && r_fr.numerator() > 0 {
            (r_fr.numerator() as f32 / r_fr.denominator() as f32).clamp(0.0, 1000.0)
        } else {
            0.0
        }
    } else if r_fr.denominator() > 0 && r_fr.numerator() > 0 {
        (r_fr.numerator() as f32 / r_fr.denominator() as f32).clamp(0.0, 1000.0)
    } else {
        0.0
    };

    let bitrate_kbps = probe_stream_bitrate_kbps(&v_stream);

    // 1. Check metadata dictionary for "rotate" tag
    if let Some(rot_str) = v_stream.metadata().get("rotate")
        && let Ok(rot_deg) = rot_str.parse::<i32>()
    {
        let norm = ((rot_deg % 360) + 360) % 360;
        if norm == 90 || norm == 180 || norm == 270 {
            rotation = norm as u32;
        }
    }
    // 2. Check stream side data for DisplayMatrix
    if rotation == 0 {
        for sd in v_stream.side_data() {
            if sd.kind() == ffmpeg_next::codec::packet::side_data::Type::DisplayMatrix {
                let data = sd.data();
                if data.len() >= 36 {
                    let m0 = i32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
                    let m1 = i32::from_ne_bytes([data[4], data[5], data[6], data[7]]);
                    let a = m0 as f64 / 65536.0;
                    let b = m1 as f64 / 65536.0;
                    let rot_deg = (-b.atan2(a).to_degrees()).round() as i32;
                    let norm = ((rot_deg % 360) + 360) % 360;
                    if norm == 90 || norm == 180 || norm == 270 {
                        rotation = norm as u32;
                    }
                }
            }
        }
    }

    let raw_w = unsafe { (*params.as_ptr()).width.max(0) as u32 };
    let raw_h = unsafe { (*params.as_ptr()).height.max(0) as u32 };
    let raw_codec_name = unsafe {
        let name_ptr = ffmpeg_next::ffi::avcodec_get_name((*params.as_ptr()).codec_id);
        if !name_ptr.is_null() {
            std::ffi::CStr::from_ptr(name_ptr)
                .to_str()
                .unwrap_or("H264")
                .to_uppercase()
        } else {
            "H264".to_string()
        }
    };

    let info = if let Ok(v_ctx) =
        ffmpeg_next::codec::context::Context::from_parameters(params.clone())
        && let Ok(v_dec) = v_ctx.decoder().video()
    {
        let w = if v_dec.width() > 0 {
            v_dec.width()
        } else {
            raw_w
        };
        let h = if v_dec.height() > 0 {
            v_dec.height()
        } else {
            raw_h
        };
        let (disp_w, disp_h) = if rotation == 90 || rotation == 270 {
            (h, w)
        } else {
            (w, h)
        };
        format!(
            "{} ({}x{})",
            v_dec
                .codec()
                .map(|c| c.name().to_string())
                .unwrap_or_else(|| raw_codec_name.clone())
                .to_uppercase(),
            disp_w,
            disp_h
        )
    } else if raw_w > 0 && raw_h > 0 {
        let (disp_w, disp_h) = if rotation == 90 || rotation == 270 {
            (raw_h, raw_w)
        } else {
            (raw_w, raw_h)
        };
        format!("{} ({}x{})", raw_codec_name, disp_w, disp_h)
    } else {
        "Unsupported Codec".to_string()
    };

    Some(VideoStreamSetup {
        stream_index,
        params,
        time_base,
        rotation,
        info,
        fps,
        bitrate_kbps,
    })
}

pub type VideoPacketChannel = (
    Sender<(u64, ffmpeg_next::Packet)>,
    Receiver<(u64, ffmpeg_next::Packet)>,
);

pub(in crate::audio) fn preferred_hw_video_devices(
    target_os: &str,
) -> &'static [(ffmpeg_next::ffi::AVHWDeviceType, &'static str)] {
    match target_os {
        "windows" => &[
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
                "HW (D3D11VA)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_DXVA2,
                "HW (DXVA2)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN,
                "HW (Vulkan)",
            ),
        ],
        "linux" => &[
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                "HW (VAAPI)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN,
                "HW (Vulkan)",
            ),
            (
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VDPAU,
                "HW (VDPAU)",
            ),
        ],
        "macos" => &[(
            ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            "HW (VideoToolbox)",
        )],
        _ => &[],
    }
}

fn open_video_decoder(
    params: &ffmpeg_next::codec::Parameters,
    allow_hw: bool,
) -> Result<(ffmpeg_next::decoder::Video, Option<&'static str>), ffmpeg_next::Error> {
    unsafe {
        ffmpeg_next::ffi::av_log_set_level(ffmpeg_next::ffi::AV_LOG_FATAL);
    }
    if allow_hw {
        let preferred_hw = preferred_hw_video_devices(std::env::consts::OS);

        let codec_ptr =
            unsafe { ffmpeg_next::ffi::avcodec_find_decoder((*params.as_ptr()).codec_id) };
        if !codec_ptr.is_null() {
            for &(hw_type, label) in preferred_hw {
                let mut hw_pix_fmt = ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
                let mut idx = 0;
                loop {
                    let cfg = unsafe { ffmpeg_next::ffi::avcodec_get_hw_config(codec_ptr, idx) };
                    if cfg.is_null() {
                        break;
                    }
                    let methods = unsafe { (*cfg).methods };
                    let dev_type = unsafe { (*cfg).device_type };
                    if (methods
                        & (ffmpeg_next::ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32))
                        != 0
                        && dev_type == hw_type
                    {
                        hw_pix_fmt = unsafe { (*cfg).pix_fmt };
                        break;
                    }
                    idx += 1;
                }

                if hw_pix_fmt == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    continue;
                }

                let mut hw_device_ctx: *mut ffmpeg_next::ffi::AVBufferRef = std::ptr::null_mut();
                let err = unsafe {
                    ffmpeg_next::ffi::av_hwdevice_ctx_create(
                        &mut hw_device_ctx,
                        hw_type,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if err == 0 && !hw_device_ctx.is_null() {
                    if let Ok(mut context) =
                        ffmpeg_next::codec::context::Context::from_parameters(params.clone())
                    {
                        unsafe {
                            (*context.as_mut_ptr()).hw_device_ctx =
                                ffmpeg_next::ffi::av_buffer_ref(hw_device_ctx);
                        }
                        if let Ok(mut decoder) = context.decoder().video() {
                            // Verify that the hardware device driver actually supports this codec,
                            // profile, and resolution (e.g. Fedora's stock Mesa VAAPI driver omits
                            // H.264/HEVC profiles unless mesa-va-drivers-freeworld is installed).
                            let frames_ok = unsafe {
                                if (*decoder.as_ptr()).sw_pix_fmt
                                    == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
                                {
                                    (*decoder.as_mut_ptr()).sw_pix_fmt = if (*decoder.as_ptr())
                                        .pix_fmt
                                        != ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
                                    {
                                        (*decoder.as_ptr()).pix_fmt
                                    } else {
                                        ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_YUV420P
                                    };
                                }
                                let mut test_frames_ctx: *mut ffmpeg_next::ffi::AVBufferRef =
                                    std::ptr::null_mut();
                                let ret = ffmpeg_next::ffi::avcodec_get_hw_frames_parameters(
                                    decoder.as_mut_ptr(),
                                    hw_device_ctx,
                                    hw_pix_fmt,
                                    &mut test_frames_ctx,
                                );
                                if !test_frames_ctx.is_null() {
                                    ffmpeg_next::ffi::av_buffer_unref(&mut test_frames_ctx);
                                }
                                ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
                                ret == 0
                            };
                            if frames_ok {
                                return Ok((decoder, Some(label)));
                            }
                            #[cfg(target_os = "linux")]
                            if hw_type == ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI {
                                // On Linux, VAAPI and Vulkan/VDPAU share the same GPU decode block
                                // and Mesa codec build config; if VAAPI opened the device and
                                // rejected the codec profile, skip redundant Vulkan/VDPAU probes.
                                break;
                            }
                        } else {
                            unsafe {
                                ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
                            }
                        }
                    } else {
                        unsafe {
                            ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
                        }
                    }
                }
            }
        }
    }

    let mut context = ffmpeg_next::codec::context::Context::from_parameters(params.clone())?;
    let hw_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .clamp(2, 16);
    context.set_threading(ffmpeg_next::codec::threading::Config {
        kind: ffmpeg_next::codec::threading::Type::Frame,
        count: hw_threads,
    });
    let decoder = context.decoder().video()?;
    Ok((decoder, None))
}

#[inline]
fn fill_plane_row_neutral(row_slice: &mut [u8], fill_u16_le: Option<[u8; 2]>, fill_byte: u8) {
    if let Some(pair) = fill_u16_le {
        let (chunks, rem) = row_slice.as_chunks_mut::<2>();
        for chunk in chunks {
            *chunk = pair;
        }
        if !rem.is_empty() {
            rem.fill(fill_byte);
        }
    } else {
        row_slice.fill(fill_byte);
    }
}

#[inline]
fn copy_ffmpeg_plane_aligned(
    src_frame: &ffmpeg_next::frame::Video,
    plane: usize,
    crop_origin: (usize, usize),
    extent: (usize, usize),
    dst: &mut Vec<u8>,
    neutral_fill: (Option<[u8; 2]>, u8),
) -> usize {
    let (crop_row, crop_byte_x) = crop_origin;
    let (row_bytes, rows) = extent;
    let (fill_u16_le, fill_byte) = neutral_fill;
    let dst_stride = crate::state::align_wgpu_row_bytes(row_bytes);
    let total = dst_stride * rows;
    dst.resize(total, fill_byte);
    if rows == 0 || row_bytes == 0 {
        return dst_stride;
    }

    let ptr = unsafe { src_frame.as_ptr() };
    if ptr.is_null() || plane >= src_frame.planes() {
        for row in 0..rows {
            let dst_start = row * dst_stride;
            fill_plane_row_neutral(
                &mut dst[dst_start..dst_start + row_bytes],
                fill_u16_le,
                fill_byte,
            );
        }
        return dst_stride;
    }

    let data_ptr = unsafe { (*ptr).data[plane] };
    let raw_linesize = unsafe { (*ptr).linesize[plane] };
    if data_ptr.is_null() || raw_linesize == 0 {
        for row in 0..rows {
            let dst_start = row * dst_stride;
            fill_plane_row_neutral(
                &mut dst[dst_start..dst_start + row_bytes],
                fill_u16_le,
                fill_byte,
            );
        }
        return dst_stride;
    }

    if raw_linesize > 0 {
        let src_stride = raw_linesize as usize;
        let src = src_frame.data(plane);
        if crop_row == 0 && crop_byte_x == 0 && src_stride == dst_stride && src.len() >= total {
            dst.copy_from_slice(&src[..total]);
            return dst_stride;
        }
        for row in 0..rows {
            let src_start = (crop_row + row)
                .saturating_mul(src_stride)
                .saturating_add(crop_byte_x);
            let dst_start = row * dst_stride;
            let dst_row = &mut dst[dst_start..dst_start + row_bytes];
            if src_start + row_bytes <= src.len() {
                dst_row.copy_from_slice(&src[src_start..src_start + row_bytes]);
            } else {
                fill_plane_row_neutral(dst_row, fill_u16_le, fill_byte);
            }
        }
    } else {
        // Negative `linesize` (bottom-up plane): `data[plane]` points to row 0 at the top of the plane buffer
        let abs_stride = raw_linesize.unsigned_abs() as usize;
        let total_plane_rows = if plane == 0 {
            src_frame.height() as usize
        } else {
            (src_frame.height() as usize).div_ceil(2)
        };
        for row in 0..rows {
            let dst_start = row * dst_stride;
            let dst_row = &mut dst[dst_start..dst_start + row_bytes];
            let src_r = crop_row + row;
            if src_r < total_plane_rows && crop_byte_x + row_bytes <= abs_stride {
                // SAFETY: `data_ptr` points to row 0 of a plane with `total_plane_rows` rows of
                // `abs_stride` bytes stepping backwards (`-abs_stride` per row).
                let row_slice = unsafe {
                    let row_ptr = data_ptr.sub(src_r * abs_stride).add(crop_byte_x);
                    std::slice::from_raw_parts(row_ptr, row_bytes)
                };
                dst_row.copy_from_slice(row_slice);
            } else {
                fill_plane_row_neutral(dst_row, fill_u16_le, fill_byte);
            }
        }
    }

    dst_stride
}

/// Packs a decoded FFmpeg `Video` frame into `frame` using 256-byte row-pitch alignment:
/// - `NV12` and `P010LE` are uploaded as 2-plane `VideoChromaLayout::SemiPlanar420` (`Y` in
///   `y_plane`, interleaved `UV` pairs in `u_plane`, empty `v_plane`), and `NV21` as
///   `VideoChromaLayout::SemiPlanarVu420`, with zero CPU scalar `UV` de-interleaving.
/// - `YUV420P` / `YUV420P10LE` / `YUV420P12LE` (or `swscale`-converted exotic formats) are
///   uploaded as 3-plane `VideoChromaLayout::Planar420`.
/// - Honors both `AVFrame` crop offsets (`crop_left`/`crop_right`/`crop_top`/`crop_bottom`) and
///   hardware surface pool padding (`active_decoded` `1920x1088` clamped to `fallback_h = 1080`).
///
/// Returns `Some(plane_tag)` on success or `None` if format conversion failed.
pub fn pack_ffmpeg_video_frame(
    active_decoded: &ffmpeg_next::frame::Video,
    fallback_w: u32,
    fallback_h: u32,
    frame: &mut crate::state::VideoFrame,
    scaler: &mut Option<(
        ffmpeg_next::format::Pixel,
        u32,
        u32,
        ffmpeg_next::software::scaling::Context,
    )>,
) -> Option<&'static str> {
    use crate::state::VideoChromaLayout;

    let coded_w = if active_decoded.width() > 0 {
        active_decoded.width()
    } else {
        fallback_w
    };
    let coded_h = if active_decoded.height() > 0 {
        active_decoded.height()
    } else {
        fallback_h
    };
    if coded_w < 2 || coded_h < 2 {
        return None;
    }

    let (raw_crop_l, raw_crop_r, raw_crop_t, raw_crop_b) = unsafe {
        let ptr = active_decoded.as_ptr();
        if ptr.is_null() {
            (0usize, 0usize, 0usize, 0usize)
        } else {
            (
                (*ptr).crop_left,
                (*ptr).crop_right,
                (*ptr).crop_top,
                (*ptr).crop_bottom,
            )
        }
    };

    let base_w = coded_w as usize;
    let base_h = coded_h as usize;
    let valid_crop_x = raw_crop_l.saturating_add(raw_crop_r) + 2 <= base_w;
    let valid_crop_y = raw_crop_t.saturating_add(raw_crop_b) + 2 <= base_h;
    let crop_left = if valid_crop_x { raw_crop_l & !1 } else { 0 };
    let crop_top = if valid_crop_y { raw_crop_t & !1 } else { 0 };
    let mut active_w = if valid_crop_x {
        base_w - crop_left - raw_crop_r
    } else {
        base_w
    };
    let mut active_h = if valid_crop_y {
        base_h - crop_top - raw_crop_b
    } else {
        base_h
    };
    if fallback_w >= 2 && (fallback_w as usize) < active_w {
        active_w = fallback_w as usize;
    }
    if fallback_h >= 2 && (fallback_h as usize) < active_h {
        active_h = fallback_h as usize;
    }
    if active_w < 2 || active_h < 2 {
        return None;
    }

    let w = active_w & !1;
    let h = active_h & !1;
    let half_w = w / 2;
    let half_h = h / 2;
    let c_crop_row = crop_top / 2;
    let c_crop_col = crop_left / 2;
    frame.width = w as u32;
    frame.height = h as u32;

    let pix_fmt = active_decoded.format();
    if (pix_fmt == ffmpeg_next::format::Pixel::NV12 || pix_fmt == ffmpeg_next::format::Pixel::NV21)
        && active_decoded.planes() >= 1
    {
        let is_nv21 = pix_fmt == ffmpeg_next::format::Pixel::NV21;
        frame.bit_depth = 8;
        frame.chroma_layout = if is_nv21 {
            VideoChromaLayout::SemiPlanarVu420
        } else {
            VideoChromaLayout::SemiPlanar420
        };
        frame.y_stride = copy_ffmpeg_plane_aligned(
            active_decoded,
            0,
            (crop_top, crop_left),
            (w, h),
            &mut frame.y_plane,
            (None, 0),
        );
        frame.u_stride = copy_ffmpeg_plane_aligned(
            active_decoded,
            1,
            (c_crop_row, c_crop_col * 2),
            (half_w * 2, half_h),
            &mut frame.u_plane,
            (None, 128),
        );
        frame.v_stride = 0;
        frame.v_plane.clear();
        return Some(if is_nv21 {
            "2-Plane NV21"
        } else {
            "2-Plane NV12"
        });
    }

    if pix_fmt == ffmpeg_next::format::Pixel::P010LE && active_decoded.planes() >= 1 {
        // MSB-aligned 10-bit surface (bits [15:6]; neutral midpoint 512 << 6 = 0x8000)
        frame.bit_depth = 16;
        frame.chroma_layout = VideoChromaLayout::SemiPlanar420;
        frame.y_stride = copy_ffmpeg_plane_aligned(
            active_decoded,
            0,
            (crop_top, crop_left * 2),
            (w * 2, h),
            &mut frame.y_plane,
            (None, 0),
        );
        frame.u_stride = copy_ffmpeg_plane_aligned(
            active_decoded,
            1,
            (c_crop_row, c_crop_col * 4),
            (half_w * 4, half_h),
            &mut frame.u_plane,
            (Some((512u16 << 6).to_le_bytes()), 0),
        );
        frame.v_stride = 0;
        frame.v_plane.clear();
        return Some("2-Plane P010");
    }

    let is_planar_420 = matches!(
        pix_fmt,
        ffmpeg_next::format::Pixel::YUV420P
            | ffmpeg_next::format::Pixel::YUVJ420P
            | ffmpeg_next::format::Pixel::YUV420P10LE
            | ffmpeg_next::format::Pixel::YUV420P12LE
    ) && active_decoded.planes() >= 1;

    let mut converted = ffmpeg_next::frame::Video::empty();
    let src_frame: &ffmpeg_next::frame::Video = if is_planar_420 {
        active_decoded
    } else {
        let sw_w = (coded_w & !1).max(2);
        let sw_h = (coded_h & !1).max(2);
        let needs_new_scaler = scaler
            .as_ref()
            .is_none_or(|(sf, sw, sh, _)| *sf != pix_fmt || *sw != sw_w || *sh != sw_h);
        if needs_new_scaler {
            *scaler = ffmpeg_next::software::scaling::Context::get(
                pix_fmt,
                sw_w,
                sw_h,
                ffmpeg_next::format::Pixel::YUV420P,
                sw_w,
                sw_h,
                ffmpeg_next::software::scaling::Flags::BILINEAR,
            )
            .ok()
            .map(|ctx| (pix_fmt, sw_w, sw_h, ctx));
        }
        if let Some((_, _, _, sws)) = scaler
            && sws.run(active_decoded, &mut converted).is_ok()
            && converted.planes() >= 3
        {
            &converted
        } else {
            return None;
        }
    };

    frame.bit_depth = match src_frame.format() {
        ffmpeg_next::format::Pixel::YUV420P10LE => 10,
        ffmpeg_next::format::Pixel::YUV420P12LE => 12,
        _ => 8,
    };
    frame.chroma_layout = VideoChromaLayout::Planar420;
    let bps = if frame.bit_depth > 8 { 2 } else { 1 };
    let chroma_fill_u16 = match frame.bit_depth {
        10 => Some(512u16.to_le_bytes()),
        12 => Some(2048u16.to_le_bytes()),
        _ => None,
    };
    let chroma_fill_byte = if bps == 1 { 128 } else { 0 };

    frame.y_stride = copy_ffmpeg_plane_aligned(
        src_frame,
        0,
        (crop_top, crop_left * bps),
        (w * bps, h),
        &mut frame.y_plane,
        (None, 0),
    );
    frame.u_stride = copy_ffmpeg_plane_aligned(
        src_frame,
        1,
        (c_crop_row, c_crop_col * bps),
        (half_w * bps, half_h),
        &mut frame.u_plane,
        (chroma_fill_u16, chroma_fill_byte),
    );
    frame.v_stride = copy_ffmpeg_plane_aligned(
        src_frame,
        2,
        (c_crop_row, c_crop_col * bps),
        (half_w * bps, half_h),
        &mut frame.v_plane,
        (chroma_fill_u16, chroma_fill_byte),
    );

    Some(match frame.bit_depth {
        10 => "3-Plane YUV420P10",
        12 => "3-Plane YUV420P12",
        _ => "3-Plane YUV420P",
    })
}

pub(in crate::audio) fn format_video_upload_telemetry(
    hw_label: Option<&str>,
    mapped_hw: bool,
    plane_tag: &str,
) -> String {
    if let Some(hw) = hw_label {
        let inner = hw
            .strip_prefix("HW (")
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(hw);
        if mapped_hw {
            format!("HW ({inner} Map -> {plane_tag})")
        } else {
            format!("HW ({inner} -> {plane_tag})")
        }
    } else {
        format!("CPU ({plane_tag})")
    }
}

pub(in crate::audio) type VideoUploadTelemetryKey = (Option<&'static str>, bool, &'static str);

#[inline]
pub(in crate::audio) fn effective_frame_hw_label(
    hw_label: Option<&'static str>,
    is_hw_frame: bool,
) -> Option<&'static str> {
    if is_hw_frame { hw_label } else { None }
}

pub(in crate::audio) fn update_video_upload_telemetry_if_changed(
    last_reported_key: &mut Option<VideoUploadTelemetryKey>,
    hw_label: Option<&'static str>,
    mapped_hw: bool,
    plane_tag: &'static str,
    on_change: impl FnOnce(String) -> bool,
) {
    let key = (hw_label, mapped_hw, plane_tag);
    if *last_reported_key != Some(key)
        && on_change(format_video_upload_telemetry(
            hw_label, mapped_hw, plane_tag,
        ))
    {
        *last_reported_key = Some(key);
    }
}

pub fn spawn_video_decoder_thread(
    params: ffmpeg_next::codec::Parameters,
    time_base: ffmpeg_next::Rational,
    rotation: u32,
    shared_state: Arc<Mutex<AppState>>,
    stop_token: Arc<std::sync::atomic::AtomicBool>,
) -> Option<VideoPacketChannel> {
    let (video_packet_tx, video_packet_rx) = bounded::<(u64, ffmpeg_next::Packet)>(4096);
    let (video_frame_tx, video_frame_rx) = bounded::<crate::state::VideoFrame>(16);
    let (free_video_frame_tx, free_video_frame_rx) = unbounded::<crate::state::VideoFrame>();
    let (init_tx, init_rx) = bounded::<bool>(1);

    for _ in 0..16 {
        let _ = free_video_frame_tx.try_send(crate::state::VideoFrame::empty(rotation));
    }

    let state_for_video = shared_state;
    let video_packet_rx_for_video = video_packet_rx.clone();
    let stop_token_video = stop_token;
    std::thread::spawn(move || {
        match open_video_decoder(&params, true) {
            Ok((mut decoder, mut hw_label)) => {
                if stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = init_tx.send(false);
                    return;
                }
                if let Ok(mut state) = state_for_video.lock() {
                    if stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = init_tx.send(false);
                        return;
                    }
                    state.video_frame_rx = Some(video_frame_rx);
                    state.free_video_frame_tx = Some(free_video_frame_tx.clone());
                    state.has_video_stream = true;
                    state.stats.video_hw_decoder = hw_label.unwrap_or("CPU").to_string();
                    #[cfg(target_os = "android")]
                    {
                        state.mobile_hud_tab = crate::state::MobileHudTab::Video;
                    }
                }
                let _ = init_tx.send(true);
                let tb = time_base.numerator() as f64 / time_base.denominator() as f64;
                let mut local_epoch = 0;
                let mut fallback_pts_seconds = 0.0;
                // Deliver the very first decoded frame immediately (even while paused at startup)
                let mut is_first_frame_after_seek = true;
                let mut decoded_any_frame = false;
                let mut pkt_bytes_accum: u64 = 0;
                let mut pkt_window_start_pts: Option<f64> = None;
                let mut prev_frame_pts: Option<f64> = None;
                let mut frame_generation: u64 = 0;
                let mut last_reported_key: Option<VideoUploadTelemetryKey> = None;
                let mut scaler: Option<(
                    ffmpeg_next::format::Pixel,
                    u32,
                    u32,
                    ffmpeg_next::software::scaling::Context,
                )> = None;

                while !stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                    match video_packet_rx_for_video
                        .recv_timeout(std::time::Duration::from_millis(50))
                    {
                        Ok((packet_epoch, packet)) => {
                            {
                                let state = state_for_video.lock().unwrap();
                                if state.seek_epoch > local_epoch {
                                    decoder.flush();
                                    local_epoch = state.seek_epoch;
                                    is_first_frame_after_seek = true;
                                    pkt_bytes_accum = 0;
                                    pkt_window_start_pts = None;
                                    prev_frame_pts = None;
                                }
                            }

                            if packet_epoch < local_epoch {
                                continue;
                            }

                            pkt_bytes_accum = pkt_bytes_accum.saturating_add(packet.size() as u64);

                            let send_t0 = std::time::Instant::now();
                            let mut send_ok = decoder.send_packet(&packet).is_ok();
                            let mut pending_send_us = send_t0.elapsed().as_micros() as f32;
                            if !send_ok
                                && hw_label.is_some()
                                && !decoded_any_frame
                                && let Ok((sw_dec, _)) = open_video_decoder(&params, false)
                            {
                                decoder = sw_dec;
                                hw_label = None;
                                last_reported_key = None;
                                if let Ok(mut state) = state_for_video.lock() {
                                    state.stats.video_hw_decoder = "CPU".to_string();
                                }
                                let sw_send_t0 = std::time::Instant::now();
                                send_ok = decoder.send_packet(&packet).is_ok();
                                pending_send_us = sw_send_t0.elapsed().as_micros() as f32;
                            }

                            if send_ok {
                                let mut decoded = ffmpeg_next::frame::Video::empty();
                                loop {
                                    let recv_t0 = std::time::Instant::now();
                                    match decoder.receive_frame(&mut decoded) {
                                        Ok(()) => {}
                                        Err(ffmpeg_next::Error::Other { errno })
                                            if errno == ffmpeg_next::ffi::EAGAIN =>
                                        {
                                            break;
                                        }
                                        Err(ffmpeg_next::Error::Eof) => break,
                                        Err(_) => {
                                            if hw_label.is_some()
                                                && !decoded_any_frame
                                                && let Ok((sw_dec, _)) =
                                                    open_video_decoder(&params, false)
                                            {
                                                decoder = sw_dec;
                                                hw_label = None;
                                                last_reported_key = None;
                                                if let Ok(mut state) = state_for_video.lock() {
                                                    state.stats.video_hw_decoder =
                                                        "CPU".to_string();
                                                }
                                                let sw_t0 = std::time::Instant::now();
                                                if decoder.send_packet(&packet).is_ok()
                                                    && decoder.receive_frame(&mut decoded).is_ok()
                                                {
                                                    pending_send_us =
                                                        sw_t0.elapsed().as_micros() as f32;
                                                } else {
                                                    break;
                                                }
                                            } else {
                                                break;
                                            }
                                        }
                                    }
                                    let codec_decode_us = std::mem::take(&mut pending_send_us)
                                        + recv_t0.elapsed().as_micros() as f32;

                                    if stop_token_video.load(std::sync::atomic::Ordering::Relaxed) {
                                        return;
                                    }
                                    let mut pts = decoded
                                        .timestamp()
                                        .map(|t| t as f64 * tb)
                                        .or_else(|| decoded.pts().map(|p| p as f64 * tb))
                                        .unwrap_or(-1.0);

                                    if pts < 0.0 {
                                        pts = fallback_pts_seconds;
                                        fallback_pts_seconds += 1.0 / 30.0;
                                    } else {
                                        fallback_pts_seconds = pts + (1.0 / 30.0);
                                    }

                                    let mut skip_push = false;
                                    if is_first_frame_after_seek {
                                        let (cached_seconds, current_epoch, duration) = {
                                            let state = state_for_video.lock().unwrap();
                                            (
                                                state.current_seconds,
                                                state.seek_epoch,
                                                state.duration_seconds,
                                            )
                                        };
                                        if current_epoch > local_epoch {
                                            skip_push = true;
                                        } else {
                                            let target_t = if duration > 0.25 {
                                                cached_seconds.min((duration - 0.25).max(0.0))
                                            } else {
                                                cached_seconds
                                            };
                                            if pts >= 0.0 && pts < target_t - 0.15 {
                                                // Fast-forward through intermediate GOP reference frames
                                                // between the keyframe and the seek target without GPU transfer
                                                // or snapping `state.current_seconds` backwards.
                                                skip_push = true;
                                            } else {
                                                is_first_frame_after_seek = false;
                                            }
                                        }
                                    } else {
                                        loop {
                                            if stop_token_video
                                                .load(std::sync::atomic::Ordering::Relaxed)
                                            {
                                                return;
                                            }
                                            let (
                                                cached_seconds,
                                                current_epoch,
                                                is_paused,
                                                track_ended,
                                            ) = {
                                                let state = state_for_video.lock().unwrap();
                                                (
                                                    state.current_seconds,
                                                    state.seek_epoch,
                                                    state.is_paused,
                                                    state.track_ended,
                                                )
                                            };

                                            if track_ended || current_epoch > local_epoch {
                                                skip_push = true;
                                                break;
                                            }

                                            if pts < cached_seconds - 0.15 {
                                                skip_push = true;
                                                break;
                                            }

                                            if pts <= cached_seconds + 0.05 {
                                                break;
                                            }

                                            // Only skip if PTS is wildly ahead due to a broken container discontinuity (>30s)
                                            if pts > cached_seconds + 30.0 {
                                                skip_push = true;
                                                break;
                                            }

                                            let sleep_dur = if is_paused { 25 } else { 4 };
                                            std::thread::sleep(std::time::Duration::from_millis(
                                                sleep_dur,
                                            ));
                                        }
                                    }

                                    if skip_push {
                                        continue;
                                    }

                                    let mut frame_opt = None;
                                    loop {
                                        if stop_token_video
                                            .load(std::sync::atomic::Ordering::Relaxed)
                                        {
                                            return;
                                        }
                                        let (current_epoch, is_paused, track_ended) = {
                                            let state = state_for_video.lock().unwrap();
                                            (state.seek_epoch, state.is_paused, state.track_ended)
                                        };
                                        if track_ended || current_epoch > local_epoch {
                                            skip_push = true;
                                            break;
                                        }
                                        match free_video_frame_rx
                                            .recv_timeout(std::time::Duration::from_millis(10))
                                        {
                                            Ok(f) => {
                                                frame_opt = Some(f);
                                                break;
                                            }
                                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                                                let sleep_dur = if is_paused { 25 } else { 5 };
                                                std::thread::sleep(
                                                    std::time::Duration::from_millis(sleep_dur),
                                                );
                                            }
                                            Err(
                                                crossbeam_channel::RecvTimeoutError::Disconnected,
                                            ) => {
                                                return;
                                            }
                                        }
                                    }

                                    if skip_push {
                                        continue;
                                    }

                                    if let Some(mut frame) = frame_opt {
                                        let transfer_start = std::time::Instant::now();
                                        let is_hw_frame =
                                            unsafe { !(*decoded.as_ptr()).hw_frames_ctx.is_null() };
                                        let mut hw_sw_frame = ffmpeg_next::frame::Video::empty();
                                        let mut mapped_hw = false;
                                        if is_hw_frame {
                                            // Attempt direct read mapping (`AV_HWFRAME_MAP_READ = 1`)
                                            // before falling back to `av_hwframe_transfer_data`.
                                            let map_ret = unsafe {
                                                ffmpeg_next::ffi::av_hwframe_map(
                                                    hw_sw_frame.as_mut_ptr(),
                                                    decoded.as_ptr(),
                                                    ffmpeg_next::ffi::AV_HWFRAME_MAP_READ as i32,
                                                )
                                            };
                                            if map_ret == 0 {
                                                mapped_hw = true;
                                                unsafe {
                                                    let dst_ptr = hw_sw_frame.as_mut_ptr();
                                                    let src_ptr = decoded.as_ptr();
                                                    if !dst_ptr.is_null()
                                                        && !src_ptr.is_null()
                                                        && (*dst_ptr).crop_top == 0
                                                        && (*dst_ptr).crop_bottom == 0
                                                        && (*dst_ptr).crop_left == 0
                                                        && (*dst_ptr).crop_right == 0
                                                    {
                                                        (*dst_ptr).crop_top = (*src_ptr).crop_top;
                                                        (*dst_ptr).crop_bottom =
                                                            (*src_ptr).crop_bottom;
                                                        (*dst_ptr).crop_left = (*src_ptr).crop_left;
                                                        (*dst_ptr).crop_right =
                                                            (*src_ptr).crop_right;
                                                    }
                                                }
                                            } else {
                                                let ret = unsafe {
                                                    ffmpeg_next::ffi::av_hwframe_transfer_data(
                                                        hw_sw_frame.as_mut_ptr(),
                                                        decoded.as_ptr(),
                                                        0,
                                                    )
                                                };
                                                if ret < 0 {
                                                    let _ = free_video_frame_tx.try_send(frame);
                                                    if hw_label.is_some()
                                                        && let Ok((sw_dec, _)) =
                                                            open_video_decoder(&params, false)
                                                    {
                                                        decoder = sw_dec;
                                                        hw_label = None;
                                                        last_reported_key = None;
                                                        if let Ok(mut state) =
                                                            state_for_video.lock()
                                                        {
                                                            state.stats.video_hw_decoder =
                                                                "CPU".to_string();
                                                        }
                                                        if !decoded_any_frame || packet.is_key() {
                                                            let _ = decoder.send_packet(&packet);
                                                        }
                                                    }
                                                    continue;
                                                }
                                            }
                                        } else if hw_label.is_some() && !decoded_any_frame {
                                            hw_label = None;
                                            last_reported_key = None;
                                            if let Ok(mut state) = state_for_video.lock() {
                                                state.stats.video_hw_decoder = "CPU".to_string();
                                            }
                                        }
                                        decoded_any_frame = true;

                                        let active_decoded: &ffmpeg_next::frame::Video =
                                            if is_hw_frame { &hw_sw_frame } else { &decoded };

                                        frame_generation = frame_generation.wrapping_add(1);
                                        frame.pts = pts;
                                        frame.rotation = rotation;
                                        frame.generation = frame_generation;
                                        frame.color_space = decoded.color_space() as u32;
                                        frame.color_range = decoded.color_range() as u32;
                                        frame.color_trc =
                                            decoded.color_transfer_characteristic() as u32;

                                        let Some(plane_tag) = pack_ffmpeg_video_frame(
                                            active_decoded,
                                            decoded.width(),
                                            decoded.height(),
                                            &mut frame,
                                            &mut scaler,
                                        ) else {
                                            let _ = free_video_frame_tx.try_send(frame);
                                            continue;
                                        };

                                        update_video_upload_telemetry_if_changed(
                                            &mut last_reported_key,
                                            effective_frame_hw_label(hw_label, is_hw_frame),
                                            mapped_hw,
                                            plane_tag,
                                            |active_path_label| {
                                                if let Ok(mut state) = state_for_video.lock() {
                                                    state.stats.video_hw_decoder =
                                                        active_path_label;
                                                    true
                                                } else {
                                                    false
                                                }
                                            },
                                        );

                                        let frame_work_us =
                                            transfer_start.elapsed().as_micros() as f32;
                                        let total_video_decode_us = codec_decode_us + frame_work_us;
                                        if let Err(crossbeam_channel::TrySendError::Full(f)) =
                                            video_frame_tx.try_send(frame)
                                        {
                                            let _ = free_video_frame_tx.try_send(f);
                                        }
                                        if let Ok(mut state) = state_for_video.try_lock() {
                                            let pkt_pct = (video_packet_rx_for_video.len() as f32
                                                / 4096.0)
                                                * 100.0;
                                            let frame_pct =
                                                (video_frame_tx.len() as f32 / 16.0) * 100.0;
                                            state.stats.video_buffer_fill_pct =
                                                pkt_pct.max(frame_pct).clamp(0.0, 100.0);
                                            if state.stats.video_decode_us <= 0.0 {
                                                state.stats.video_decode_us = total_video_decode_us;
                                            } else {
                                                state.stats.video_decode_us =
                                                    state.stats.video_decode_us * 0.9
                                                        + total_video_decode_us * 0.1;
                                            }
                                            if state.stats.video_fps <= 0.0
                                                && let Some(p_pts) = prev_frame_pts
                                            {
                                                let dt = pts - p_pts;
                                                if (0.001..0.5).contains(&dt) {
                                                    state.stats.video_fps = (1.0 / dt) as f32;
                                                }
                                            }
                                            if pts >= 0.0 {
                                                prev_frame_pts = Some(pts);
                                                if let Some(start_pts) = pkt_window_start_pts {
                                                    let win_dur = pts - start_pts;
                                                    if win_dur >= 0.5 {
                                                        let kbps = ((pkt_bytes_accum as f64 * 8.0)
                                                            / (win_dur * 1000.0))
                                                            .round()
                                                            as u32;
                                                        if kbps > 0
                                                            && state
                                                                .stats
                                                                .video_bitrate_kbps
                                                                .is_none()
                                                        {
                                                            state.stats.video_bitrate_kbps =
                                                                Some(kbps);
                                                        }
                                                        pkt_window_start_pts = Some(pts);
                                                        pkt_bytes_accum = 0;
                                                    }
                                                } else {
                                                    pkt_window_start_pts = Some(pts);
                                                    pkt_bytes_accum = 0;
                                                }
                                            }
                                            if state.stats.bitstream_active {
                                                state.stats.decode_us =
                                                    if state.stats.decode_us <= 0.0 {
                                                        total_video_decode_us
                                                    } else {
                                                        state.stats.decode_us * 0.9
                                                            + total_video_decode_us * 0.1
                                                    };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[video] Failed to open FFmpeg video decoder ({:?}): {}; falling back to audio-only playback",
                    params.id(),
                    e
                );
                #[cfg(target_os = "android")]
                crate::android::log_android(
                    5,
                    &format!(
                        "[RustTracker Video] FFmpeg video decoder failed ({:?}): {}; falling back to audio-only",
                        params.id(),
                        e
                    ),
                );
                if !stop_token_video.load(std::sync::atomic::Ordering::Relaxed)
                    && let Ok(mut state) = state_for_video.lock()
                    && !stop_token_video.load(std::sync::atomic::Ordering::Relaxed)
                {
                    state.has_video_stream = false;
                    state.video_frame_rx = None;
                    state.free_video_frame_tx = None;
                    state.video_info = None;
                    state.stats.video_buffer_fill_pct = 0.0;
                    state.stats.video_hw_decoder.clear();
                    #[cfg(target_os = "android")]
                    if state.mobile_hud_tab == crate::state::MobileHudTab::Video {
                        state.mobile_hud_tab = crate::state::MobileHudTab::Channels;
                    }
                }
                let _ = init_tx.send(false);
            }
        }
    });

    if init_rx.recv().unwrap_or(false) {
        Some((video_packet_tx, video_packet_rx))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::audio) fn spawn_media_decoder_threads(
    mut audio_source: Box<dyn AudioSource>,
    shared_state: Arc<Mutex<AppState>>,
    stop_token: Arc<std::sync::atomic::AtomicBool>,
    mut ready_tx: rtrb::Producer<AudioChunk>,
    mut free_rx: rtrb::Consumer<AudioChunk>,
    hardware_channels: usize,
    sample_rate: u32,
    chunk_frames: usize,
    pool_size: usize,
    prime_chunks: usize,
    _current_path: &str,
    stream_shutdown_rx: Option<Receiver<()>>,
) {
    #[cfg(target_os = "android")]
    let android_hw_video_started = if audio_source.has_video_stream() {
        crate::android_video::decoder::start_android_video_thread(
            _current_path,
            shared_state.clone(),
            stop_token.clone(),
        )
    } else {
        false
    };
    #[cfg(not(target_os = "android"))]
    let android_hw_video_started = false;

    let video_packet_rx = if !android_hw_video_started
        && let Some((params, time_base, rotation)) = audio_source.take_video_parameters()
        && let Some((video_packet_tx, video_packet_rx)) = spawn_video_decoder_thread(
            params,
            time_base,
            rotation,
            shared_state.clone(),
            stop_token.clone(),
        ) {
        audio_source.attach_video_queue(video_packet_tx);
        video_packet_rx
    } else {
        let (_tx, rx) = bounded::<(u64, ffmpeg_next::Packet)>(1);
        rx
    };

    let mut initial_spare_chunk: Option<AudioChunk> = None;
    for _ in 0..prime_chunks {
        if let Ok(mut chunk) = free_rx.pop() {
            let (frames_read, _, _) = decode_single_audio_chunk(
                audio_source.as_mut(),
                &mut chunk,
                hardware_channels,
                sample_rate,
                chunk_frames,
            );
            if frames_read > 0 {
                if !stop_token.load(std::sync::atomic::Ordering::Relaxed)
                    && let Ok(mut state) = shared_state.lock()
                    && !stop_token.load(std::sync::atomic::Ordering::Relaxed)
                {
                    if state.bitrate.is_none() {
                        state.bitrate = audio_source.get_bitrate();
                        state.stats.audio_bitrate_kbps = state.bitrate;
                    }
                    chunk.seek_epoch = state.seek_epoch;
                    push_chunk_lookahead_slices(
                        &mut state,
                        &chunk,
                        frames_read,
                        hardware_channels,
                        sample_rate,
                    );
                    state.sync_rt_control();
                }
                chunk.track_ended = false;
                if let Err(rtrb::PushError::Full(rejected)) = ready_tx.push(chunk) {
                    initial_spare_chunk = Some(rejected);
                    break;
                }
            } else {
                initial_spare_chunk = Some(chunk);
                break;
            }
        }
    }

    let state_for_decoder = shared_state.clone();
    let video_rx_for_decoder = video_packet_rx.clone();
    let stop_token_decoder = stop_token.clone();

    std::thread::spawn(move || {
        let mut just_seeked = false;
        let mut spare_chunk = initial_spare_chunk;
        while !stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed) {
            let mut chunk = if let Some(c) = spare_chunk.take() {
                c
            } else {
                loop {
                    if stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed) {
                        spare_chunk = None;
                        break;
                    }
                    if let Ok(mut state) = state_for_decoder.try_lock() {
                        state.sync_rt_control();
                        if process_decoder_state_requests(&mut state, audio_source.as_mut(), || {
                            while video_rx_for_decoder.try_recv().is_ok() {}
                        }) {
                            just_seeked = true;
                        }
                    }
                    match free_rx.pop() {
                        Ok(c) => {
                            spare_chunk = Some(c);
                            break;
                        }
                        Err(_) => {
                            if free_rx.is_abandoned() {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                    }
                }
                let Some(c) = spare_chunk.take() else {
                    break;
                };
                c
            };

            let (frames_read, decode_elapsed, clips) = decode_single_audio_chunk(
                audio_source.as_mut(),
                &mut chunk,
                hardware_channels,
                sample_rate,
                chunk_frames,
            );

            let current_artist = audio_source.get_artist();
            let current_bitrate = audio_source.get_bitrate();
            if !stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed)
                && let Ok(mut state) = state_for_decoder.lock()
                && !stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed)
            {
                if just_seeked {
                    if frames_read > 0
                        && (state.current_seconds - chunk.current_seconds).abs() < 0.5
                    {
                        state.current_seconds = chunk.current_seconds;
                    }
                    just_seeked = false;
                }
                chunk.seek_epoch = state.seek_epoch;
                if state.bitrate.is_none() && current_bitrate.is_some() {
                    state.bitrate = current_bitrate;
                    state.stats.audio_bitrate_kbps = current_bitrate;
                }
                if current_artist != "Unknown" && !current_artist.is_empty() {
                    if let Some((artist_part, title_part)) = current_artist.split_once(" - ") {
                        if state.artist != artist_part || state.song_title != title_part {
                            state.artist = artist_part.to_string();
                            state.song_title = title_part.to_string();
                        }
                    } else if state.artist != current_artist {
                        state.artist = current_artist.clone();
                    }
                }
                state.stats.decode_us = state.stats.decode_us * 0.9 + decode_elapsed * 0.1;
                let queued = pool_size.saturating_sub(ready_tx.slots());
                let fill_pct = (queued as f32 / pool_size.max(1) as f32) * 100.0;
                state.stats.audio_buffer_fill_pct = fill_pct;
                let pkt_pct = (video_rx_for_decoder.len() as f32 / 4096.0) * 100.0;
                let frame_pct = state
                    .video_frame_rx
                    .as_ref()
                    .map(|rx| (rx.len() as f32 / 16.0) * 100.0)
                    .unwrap_or(0.0);
                state.stats.video_buffer_fill_pct = pkt_pct.max(frame_pct).clamp(0.0, 100.0);
                state.stats.clipping_events += clips;
                push_chunk_lookahead_slices(
                    &mut state,
                    &chunk,
                    frames_read,
                    hardware_channels,
                    sample_rate,
                );
                state.sync_rt_control();
            }

            let mut chunk_to_send = chunk;
            let mut abandoned = false;
            loop {
                if stop_token_decoder.load(std::sync::atomic::Ordering::Relaxed)
                    || ready_tx.is_abandoned()
                {
                    drop(chunk_to_send);
                    abandoned = true;
                    break;
                }
                let mut seeked = false;
                if let Ok(mut state) = state_for_decoder.try_lock() {
                    state.sync_rt_control();
                    seeked =
                        process_decoder_state_requests(&mut state, audio_source.as_mut(), || {
                            while video_rx_for_decoder.try_recv().is_ok() {}
                        });
                }
                if seeked {
                    just_seeked = true;
                    spare_chunk = Some(chunk_to_send);
                    break;
                }
                match ready_tx.push(chunk_to_send) {
                    Ok(()) => break,
                    Err(rtrb::PushError::Full(c)) => {
                        chunk_to_send = c;
                        std::thread::sleep(std::time::Duration::from_millis(4));
                    }
                }
            }
            if abandoned {
                break;
            }

            if frames_read == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        // Keep `free_rx` and `ready_tx` alive until the consumer / CPAL RT callback has
        // completely stopped so remaining pooled `AudioChunk`s are always freed on this thread.
        if let Some(rx) = stream_shutdown_rx {
            let _ = rx.recv();
        }
        drop(spare_chunk);
        drop(free_rx);
        drop(ready_tx);
    });
}
