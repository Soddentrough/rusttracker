/// Maps Android `MediaFormat` `KEY_COLOR_STANDARD` values to FFmpeg `AVColorSpace` codes
/// consumed by `src/shaders/vis_video.wgsl`.
pub fn map_android_color_standard(color_standard: Option<i32>, height: u32) -> u32 {
    match color_standard {
        // COLOR_STANDARD_BT709 = 1 -> AVCOL_SPC_BT709 = 1
        Some(1) => 1,
        // COLOR_STANDARD_BT601_PAL = 2 -> AVCOL_SPC_BT470BG = 5
        Some(2) => 5,
        // COLOR_STANDARD_BT601_NTSC = 4 -> AVCOL_SPC_SMPTE170M = 6
        Some(4) => 6,
        // COLOR_STANDARD_BT2020 = 6 -> AVCOL_SPC_BT2020_NCL = 9
        Some(6) => 9,
        _ => {
            if height < 720 {
                5
            } else {
                1
            }
        }
    }
}

/// Maps Android `MediaFormat` `KEY_COLOR_RANGE` values to FFmpeg `AVColorRange` codes.
pub fn map_android_color_range(color_range: Option<i32>) -> u32 {
    match color_range {
        // COLOR_RANGE_FULL = 1 -> AVCOL_RANGE_JPEG = 2
        Some(1) => 2,
        // COLOR_RANGE_LIMITED = 2 -> AVCOL_RANGE_MPEG = 1
        Some(2) => 1,
        // Default YUV video range is limited (TV)
        _ => 1,
    }
}

/// Maps Android `MediaFormat` `KEY_COLOR_TRANSFER` values to FFmpeg `AVColorTransferCharacteristic`.
pub fn map_android_color_transfer(color_transfer: Option<i32>) -> u32 {
    match color_transfer {
        // COLOR_TRANSFER_LINEAR = 1 -> AVCOL_TRC_LINEAR = 8
        Some(1) => 8,
        // COLOR_TRANSFER_SDR_VIDEO = 3 -> AVCOL_TRC_BT709 = 1
        Some(3) => 1,
        // COLOR_TRANSFER_ST2084 (PQ) = 6 -> AVCOL_TRC_SMPTE2084 = 16
        Some(6) => 16,
        // COLOR_TRANSFER_HLG = 7 -> AVCOL_TRC_ARIB_STD_B67 = 18
        Some(7) => 18,
        _ => 1,
    }
}

/// Returns preferred `AMediaCodec` output `color-format` negotiation order:
/// - `[54, 21, 19]` (`COLOR_FormatYUVP010` first, then NV12, then I420) for 10-bit / HDR streams
///   so Main10 / HDR10 / HLG content preserves 10-bit precision instead of failing or truncating;
/// - `[21, 19, 54]` (NV12 -> I420 -> P010) for 8-bit SDR streams.
pub fn preferred_android_color_formats(
    mime: &str,
    profile: Option<i32>,
    bits_per_sample: Option<i32>,
    color_space: u32,
    color_trc: u32,
) -> [i32; 3] {
    let profile_is_10bit = match (mime, profile) {
        // HEVC Main10 (2), Main10HDR10 (0x1000), Main10HDR10Plus (0x2000)
        ("video/hevc", Some(p)) => p == 2 || (p & 0x3000) != 0,
        // AV1 Main10 (2), Main10HDR10 (0x1000), Main10HDR10Plus (0x2000)
        ("video/av01", Some(p)) => p == 2 || (p & 0x3000) != 0,
        // VP9 Profile2 (4), Profile3 (8), Profile2HDR (0x1000), Profile2HDR10Plus (0x4000)
        ("video/x-vnd.on2.vp9", Some(p)) => p == 4 || p == 8 || (p & 0x5000) != 0,
        // AVC High10 (0x10)
        ("video/avc", Some(p)) => p == 0x10,
        _ => false,
    };
    let is_hdr_or_10bit = profile_is_10bit
        || bits_per_sample.is_some_and(|b| b >= 10)
        || color_space == 9
        || color_trc == 16
        || color_trc == 18;
    if is_hdr_or_10bit {
        [54, 21, 19]
    } else {
        [21, 19, 54]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AndroidYuvLayout {
    /// Planar 8-bit 4:2:0 I420 (`COLOR_FormatYUV420Planar` = 19 / `COLOR_FormatYUV420PackedPlanar` = 20)
    Planar420,
    /// Planar 8-bit 4:2:0 YV12 (`HAL_PIXEL_FORMAT_YV12` = `0x32315659`, Y + V + U with 16B-aligned chroma stride)
    PlanarYv12,
    /// Semi-planar 8-bit NV12 (`COLOR_FormatYUV420SemiPlanar` = 21 / `39` / QCOM `0x7FA30C04`)
    SemiPlanarNv12,
    /// Semi-planar 8-bit NV21 (`COLOR_FormatYUV420SemiPlanar` VU = 17 / `0x7F000100`)
    SemiPlanarNv21,
    /// Semi-planar 10-bit MSB-aligned P010LE (`COLOR_FormatYUVP010` = 54)
    SemiPlanarP010,
}

impl AndroidYuvLayout {
    pub fn from_color_format(color_format: i32) -> Self {
        match color_format {
            19 | 20 => Self::Planar420,
            0x3231_5659 => Self::PlanarYv12,
            17 | 0x7F00_0100 => Self::SemiPlanarNv21,
            54 => Self::SemiPlanarP010,
            _ => Self::SemiPlanarNv12,
        }
    }

    pub fn bytes_per_sample(self) -> usize {
        match self {
            Self::SemiPlanarP010 => 2,
            Self::Planar420 | Self::PlanarYv12 | Self::SemiPlanarNv12 | Self::SemiPlanarNv21 => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AndroidBufferGeometry {
    pub coded_w: usize,
    pub coded_h: usize,
    pub crop_left: usize,
    pub crop_top: usize,
    pub active_w: usize,
    pub active_h: usize,
    pub stride_bytes: usize,
    pub slice_height: usize,
    pub layout: AndroidYuvLayout,
}

pub fn resolve_android_buffer_geometry(
    coded_w: i32,
    coded_h: i32,
    crop_rect: Option<(i32, i32, i32, i32)>,
    fmt_stride: i32,
    fmt_slice_height: i32,
    color_format: i32,
) -> AndroidBufferGeometry {
    let layout = AndroidYuvLayout::from_color_format(color_format);
    let sample_bytes = layout.bytes_per_sample();
    let cw = coded_w.max(2) as usize;
    let ch = coded_h.max(2) as usize;

    let (crop_left, crop_top, active_w, active_h) = if let Some((left, top, right, bottom)) =
        crop_rect
        && left >= 0
        && top >= 0
        && right >= left
        && bottom >= top
        && (right as usize) < cw
        && (bottom as usize) < ch
    {
        // Align crop origin to 2x2 4:2:0 subsampling boundaries
        let cl = (left as usize) & !1;
        let ct = (top as usize) & !1;
        let aw = ((right as usize + 1).saturating_sub(cl)).max(2) & !1;
        let ah = ((bottom as usize + 1).saturating_sub(ct)).max(2) & !1;
        (cl, ct, aw.max(2), ah.max(2))
    } else {
        let aw = (cw & !1).max(2);
        let ah = (ch & !1).max(2);
        (0, 0, aw, ah)
    };

    let min_row_bytes = cw * sample_bytes;
    let stride_bytes = if fmt_stride > 0 && (fmt_stride as usize) >= min_row_bytes {
        fmt_stride as usize
    } else if sample_bytes > 1 && fmt_stride > 0 && (fmt_stride as usize) >= cw {
        // Vendor codec reported KEY_STRIDE in luma samples (pixels) instead of bytes on P010
        (fmt_stride as usize) * sample_bytes
    } else {
        min_row_bytes
    };
    let slice_height = if fmt_slice_height > 0 && (fmt_slice_height as usize) >= ch {
        fmt_slice_height as usize
    } else {
        ch
    };

    AndroidBufferGeometry {
        coded_w: cw,
        coded_h: ch,
        crop_left,
        crop_top,
        active_w,
        active_h,
        stride_bytes,
        slice_height,
        layout,
    }
}

pub fn android_hw_decoder_label(layout: AndroidYuvLayout) -> &'static str {
    match layout {
        AndroidYuvLayout::SemiPlanarNv12 => "HW (MediaCodec -> 2-Plane NV12)",
        AndroidYuvLayout::SemiPlanarNv21 => "HW (MediaCodec -> 2-Plane NV21)",
        AndroidYuvLayout::SemiPlanarP010 => "HW (MediaCodec -> 2-Plane P010)",
        AndroidYuvLayout::Planar420 => "HW (MediaCodec -> 3-Plane I420)",
        AndroidYuvLayout::PlanarYv12 => "HW (MediaCodec -> 3-Plane YV12)",
    }
}

/// Unpacks a raw `AMediaCodec` output buffer into 256-byte-row-aligned `VideoFrame` planes,
/// honoring crop offsets, row `stride`, `slice-height`, and `color-format`:
/// - `NV12` (`SemiPlanarNv12`), `NV21` (`SemiPlanarNv21`), and `P010` (`SemiPlanarP010`) keep
///   interleaved chroma pairs in `frame.u_plane` (`SemiPlanar420` / `SemiPlanarVu420`) via row
///   `copy_from_slice` with zero CPU scalar `UV` de-interleaving and zero `v_plane` allocation.
/// - `I420` (`Planar420`) and `YV12` (`PlanarYv12`, `Y`+`V`+`U` with 16-byte chroma stride
///   alignment) pack 3-plane `Y`/`U`/`V` (`VideoChromaLayout::Planar420`).
pub fn unpack_android_yuv_frame(
    src_slice: &[u8],
    geom: &AndroidBufferGeometry,
    frame: &mut rt_core::state::VideoFrame,
) {
    use rt_core::state::{VideoChromaLayout, align_wgpu_row_bytes};

    let w = geom.active_w;
    let h = geom.active_h;
    let half_w = w / 2;
    let half_h = h / 2;
    frame.width = w as u32;
    frame.height = h as u32;

    match geom.layout {
        AndroidYuvLayout::SemiPlanarNv12 | AndroidYuvLayout::SemiPlanarNv21 => {
            let y_row_bytes = w;
            let uv_row_bytes = half_w * 2;
            let y_stride = align_wgpu_row_bytes(y_row_bytes);
            let uv_stride = align_wgpu_row_bytes(uv_row_bytes);

            frame.bit_depth = 8;
            frame.chroma_layout = if geom.layout == AndroidYuvLayout::SemiPlanarNv21 {
                VideoChromaLayout::SemiPlanarVu420
            } else {
                VideoChromaLayout::SemiPlanar420
            };
            frame.y_stride = y_stride;
            frame.u_stride = uv_stride;
            frame.v_stride = 0;

            frame.y_plane.resize(y_stride * h, 0);
            frame.u_plane.resize(uv_stride * half_h, 128);
            frame.v_plane.clear();

            for row in 0..h {
                let src_offset = (geom.crop_top + row) * geom.stride_bytes + geom.crop_left;
                let dst_offset = row * y_stride;
                if src_offset + y_row_bytes <= src_slice.len() {
                    frame.y_plane[dst_offset..dst_offset + y_row_bytes]
                        .copy_from_slice(&src_slice[src_offset..src_offset + y_row_bytes]);
                } else {
                    frame.y_plane[dst_offset..dst_offset + y_row_bytes].fill(0);
                }
            }

            let uv_base = geom.stride_bytes * geom.slice_height;
            let uv_crop_row = geom.crop_top / 2;
            let uv_crop_byte_x = (geom.crop_left / 2) * 2;
            for row in 0..half_h {
                let dst_row = row * uv_stride;
                let src_row = uv_base
                    .saturating_add((uv_crop_row + row).saturating_mul(geom.stride_bytes))
                    .saturating_add(uv_crop_byte_x);
                if uv_base < src_slice.len() && src_row + uv_row_bytes <= src_slice.len() {
                    frame.u_plane[dst_row..dst_row + uv_row_bytes]
                        .copy_from_slice(&src_slice[src_row..src_row + uv_row_bytes]);
                } else {
                    frame.u_plane[dst_row..dst_row + uv_row_bytes].fill(128);
                }
            }
        }
        AndroidYuvLayout::Planar420 | AndroidYuvLayout::PlanarYv12 => {
            let y_stride = align_wgpu_row_bytes(w);
            let c_stride = align_wgpu_row_bytes(half_w);
            let is_yv12 = geom.layout == AndroidYuvLayout::PlanarYv12;

            frame.bit_depth = 8;
            frame.chroma_layout = VideoChromaLayout::Planar420;
            frame.y_stride = y_stride;
            frame.u_stride = c_stride;
            frame.v_stride = c_stride;

            frame.y_plane.resize(y_stride * h, 0);
            frame.u_plane.resize(c_stride * half_h, 128);
            frame.v_plane.resize(c_stride * half_h, 128);

            for row in 0..h {
                let src_offset = (geom.crop_top + row) * geom.stride_bytes + geom.crop_left;
                let dst_offset = row * y_stride;
                if src_offset + w <= src_slice.len() {
                    frame.y_plane[dst_offset..dst_offset + w]
                        .copy_from_slice(&src_slice[src_offset..src_offset + w]);
                } else {
                    frame.y_plane[dst_offset..dst_offset + w].fill(0);
                }
            }

            // Android HAL_PIXEL_FORMAT_YV12 aligns chroma row stride to 16 bytes and orders planes Y -> V -> U
            let chroma_stride = if is_yv12 {
                ((geom.stride_bytes / 2) + 15) & !15
            } else {
                geom.stride_bytes / 2
            };
            let chroma_slice_h = geom.slice_height / 2;
            let first_chroma_base = geom.stride_bytes * geom.slice_height;
            let second_chroma_base = first_chroma_base + chroma_stride * chroma_slice_h;
            let (u_base, v_base) = if is_yv12 {
                (second_chroma_base, first_chroma_base)
            } else {
                (first_chroma_base, second_chroma_base)
            };
            let c_crop_row = geom.crop_top / 2;
            let c_crop_col = geom.crop_left / 2;

            for row in 0..half_h {
                let dst_offset = row * c_stride;
                let u_src = u_base + (c_crop_row + row) * chroma_stride + c_crop_col;
                if u_src + half_w <= src_slice.len() {
                    frame.u_plane[dst_offset..dst_offset + half_w]
                        .copy_from_slice(&src_slice[u_src..u_src + half_w]);
                } else {
                    frame.u_plane[dst_offset..dst_offset + half_w].fill(128);
                }
                let v_src = v_base + (c_crop_row + row) * chroma_stride + c_crop_col;
                if v_src + half_w <= src_slice.len() {
                    frame.v_plane[dst_offset..dst_offset + half_w]
                        .copy_from_slice(&src_slice[v_src..v_src + half_w]);
                } else {
                    frame.v_plane[dst_offset..dst_offset + half_w].fill(128);
                }
            }
        }
        AndroidYuvLayout::SemiPlanarP010 => {
            let y_row_bytes = w * 2;
            let uv_row_bytes = half_w * 4;
            let y_stride = align_wgpu_row_bytes(y_row_bytes);
            let uv_stride = align_wgpu_row_bytes(uv_row_bytes);

            frame.bit_depth = 16; // MSB-aligned 10-bit P010LE
            frame.chroma_layout = VideoChromaLayout::SemiPlanar420;
            frame.y_stride = y_stride;
            frame.u_stride = uv_stride;
            frame.v_stride = 0;

            frame.y_plane.resize(y_stride * h, 0);
            frame.u_plane.resize(uv_stride * half_h, 0);
            frame.v_plane.clear();

            let crop_x_bytes = geom.crop_left * 2;
            for row in 0..h {
                let src_offset = (geom.crop_top + row) * geom.stride_bytes + crop_x_bytes;
                let dst_offset = row * y_stride;
                if src_offset + y_row_bytes <= src_slice.len() {
                    frame.y_plane[dst_offset..dst_offset + y_row_bytes]
                        .copy_from_slice(&src_slice[src_offset..src_offset + y_row_bytes]);
                } else {
                    frame.y_plane[dst_offset..dst_offset + y_row_bytes].fill(0);
                }
            }

            let uv_base = geom.stride_bytes * geom.slice_height;
            let uv_crop_row = geom.crop_top / 2;
            let uv_crop_byte_x = (geom.crop_left / 2) * 4;
            for row in 0..half_h {
                let dst_row = row * uv_stride;
                let uv_out = &mut frame.u_plane[dst_row..dst_row + uv_row_bytes];
                let src_row = uv_base
                    .saturating_add((uv_crop_row + row).saturating_mul(geom.stride_bytes))
                    .saturating_add(uv_crop_byte_x);
                if uv_base < src_slice.len() && src_row + uv_row_bytes <= src_slice.len() {
                    uv_out.copy_from_slice(&src_slice[src_row..src_row + uv_row_bytes]);
                } else {
                    // Neutral 10-bit MSB-aligned chroma (512 << 6 = 0x8000 -> [0x00, 0x80] LE)
                    for pair in uv_out.as_chunks_mut::<2>().0 {
                        *pair = [0x00, 0x80];
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "android")]
pub mod decoder {
    use super::{
        android_hw_decoder_label, map_android_color_range, map_android_color_standard,
        map_android_color_transfer, preferred_android_color_formats,
        resolve_android_buffer_geometry, unpack_android_yuv_frame,
    };
    use crossbeam_channel::{bounded, unbounded};
    use rt_core::state::{AppState, VideoFrame};
    use std::ffi::{CStr, c_char, c_void};
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[repr(C)]
    struct AMediaExtractor {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct AMediaCodec {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct AMediaFormat {
        _private: [u8; 0],
    }

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
        fn AMediaExtractor_setDataSourceFd(
            extractor: *mut AMediaExtractor,
            fd: i32,
            offset: i64,
            length: i64,
        ) -> i32;
        fn AMediaExtractor_getTrackCount(extractor: *mut AMediaExtractor) -> usize;
        fn AMediaExtractor_getTrackFormat(
            extractor: *mut AMediaExtractor,
            idx: usize,
        ) -> *mut AMediaFormat;
        fn AMediaExtractor_selectTrack(extractor: *mut AMediaExtractor, idx: usize) -> i32;
        fn AMediaExtractor_seekTo(
            extractor: *mut AMediaExtractor,
            seekPosUs: i64,
            mode: i32,
        ) -> i32;
        fn AMediaExtractor_advance(extractor: *mut AMediaExtractor) -> bool;
        fn AMediaExtractor_readSampleData(
            extractor: *mut AMediaExtractor,
            buffer: *mut u8,
            capacity: usize,
        ) -> isize;
        fn AMediaExtractor_getSampleTime(extractor: *mut AMediaExtractor) -> i64;
        fn AMediaExtractor_getSampleFlags(extractor: *mut AMediaExtractor) -> u32;

        fn AMediaFormat_delete(format: *mut AMediaFormat) -> i32;
        fn AMediaFormat_getString(
            format: *mut AMediaFormat,
            name: *const c_char,
            out: *mut *const c_char,
        ) -> bool;
        fn AMediaFormat_getInt32(
            format: *mut AMediaFormat,
            name: *const c_char,
            out: *mut i32,
        ) -> bool;
        fn AMediaFormat_setInt32(format: *mut AMediaFormat, name: *const c_char, value: i32);

        fn AMediaCodec_createDecoderByType(mime_type: *const c_char) -> *mut AMediaCodec;
        fn AMediaCodec_configure(
            codec: *mut AMediaCodec,
            format: *const AMediaFormat,
            surface: *mut c_void,
            crypto: *mut c_void,
            flags: u32,
        ) -> i32;
        fn AMediaCodec_start(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_stop(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_delete(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_flush(codec: *mut AMediaCodec) -> i32;
        fn AMediaCodec_dequeueInputBuffer(codec: *mut AMediaCodec, timeoutUs: i64) -> isize;
        fn AMediaCodec_getInputBuffer(
            codec: *mut AMediaCodec,
            idx: usize,
            out_size: *mut usize,
        ) -> *mut u8;
        fn AMediaCodec_queueInputBuffer(
            codec: *mut AMediaCodec,
            idx: usize,
            offset: usize,
            size: usize,
            pts: u64,
            flags: u32,
        ) -> i32;
        fn AMediaCodec_dequeueOutputBuffer(
            codec: *mut AMediaCodec,
            info: *mut AMediaCodecBufferInfo,
            timeoutUs: i64,
        ) -> isize;
        fn AMediaCodec_getOutputBuffer(
            codec: *mut AMediaCodec,
            idx: usize,
            out_size: *mut usize,
        ) -> *mut u8;
        fn AMediaCodec_getOutputFormat(codec: *mut AMediaCodec) -> *mut AMediaFormat;
        fn AMediaCodec_releaseOutputBuffer(
            codec: *mut AMediaCodec,
            idx: usize,
            render: bool,
        ) -> i32;
    }

    unsafe fn get_opt_int32(format: *const AMediaFormat, key: &CStr) -> Option<i32> {
        let mut val: i32 = 0;
        if unsafe { AMediaFormat_getInt32(format as *mut AMediaFormat, key.as_ptr(), &mut val) } {
            Some(val)
        } else {
            None
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FormatSnapshot {
        geom: super::AndroidBufferGeometry,
        rotation: u32,
        color_space: u32,
        color_range: u32,
        color_trc: u32,
    }

    unsafe fn read_format_snapshot(
        format: *const AMediaFormat,
        prev: Option<FormatSnapshot>,
    ) -> FormatSnapshot {
        let coded_w = unsafe { get_opt_int32(format, c"width") }
            .unwrap_or_else(|| prev.map(|p| p.geom.coded_w as i32).unwrap_or(1920));
        let coded_h = unsafe { get_opt_int32(format, c"height") }
            .unwrap_or_else(|| prev.map(|p| p.geom.coded_h as i32).unwrap_or(1080));
        let stride = unsafe { get_opt_int32(format, c"stride") }.unwrap_or(0);
        let slice_h = unsafe { get_opt_int32(format, c"slice-height") }.unwrap_or(0);
        let color_fmt = unsafe { get_opt_int32(format, c"color-format") }.unwrap_or(21);

        let crop_rect = match (
            unsafe { get_opt_int32(format, c"crop-left") },
            unsafe { get_opt_int32(format, c"crop-top") },
            unsafe { get_opt_int32(format, c"crop-right") },
            unsafe { get_opt_int32(format, c"crop-bottom") },
        ) {
            (Some(l), Some(t), Some(r), Some(b)) => Some((l, t, r, b)),
            _ => None,
        };

        let geom = resolve_android_buffer_geometry(
            coded_w, coded_h, crop_rect, stride, slice_h, color_fmt,
        );

        let rotation = if let Some(rot) = unsafe { get_opt_int32(format, c"rotation-degrees") } {
            match rot.rem_euclid(360) {
                90 | 180 | 270 => rot.rem_euclid(360) as u32,
                _ => 0,
            }
        } else {
            prev.map(|p| p.rotation).unwrap_or(0)
        };

        let raw_std = unsafe { get_opt_int32(format, c"color-standard") };
        let color_space = match (raw_std, prev) {
            (None, Some(p)) => p.color_space,
            _ => map_android_color_standard(raw_std, geom.active_h as u32),
        };

        let raw_range = unsafe { get_opt_int32(format, c"color-range") };
        let color_range = match (raw_range, prev) {
            (None, Some(p)) => p.color_range,
            _ => map_android_color_range(raw_range),
        };

        let raw_trc = unsafe { get_opt_int32(format, c"color-transfer") };
        let color_trc = match (raw_trc, prev) {
            (None, Some(p)) => p.color_trc,
            _ => map_android_color_transfer(raw_trc),
        };

        FormatSnapshot {
            geom,
            rotation,
            color_space,
            color_range,
            color_trc,
        }
    }

    /// Attempts to start a hardware `AMediaExtractor` + `AMediaCodec` video decoder for `path`.
    /// Returns `true` ONLY if the container and video stream were accepted and `AMediaCodec_start`
    /// succeeded; returns `false` cleanly (without touching `state.video_frame_rx`) when `path` is
    /// a network URL, `.iso` disc image, unsupported container (e.g. `.avi`/`.vob`), or unsupported
    /// codec so the caller can fall back to static FFmpeg decoding.
    pub fn start_android_video_thread(
        path: &str,
        shared_state: Arc<Mutex<AppState>>,
        stop_token: Arc<AtomicBool>,
    ) -> bool {
        let lower = path.to_ascii_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.ends_with(".iso")
            || lower.ends_with(".vob")
            || lower.ends_with(".aob")
        {
            return false;
        }

        let path_string = path.to_string();
        let (init_tx, init_rx) = bounded::<bool>(1);

        let spawn_ok = std::thread::Builder::new()
            .name("AndroidVideoDecoder".to_string())
            .spawn(move || {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                    let extractor = AMediaExtractor_new();
                    if extractor.is_null() {
                        rt_core::android_log::log_android(
                            5,
                            "[RustTracker Video] Failed to create AMediaExtractor",
                        );
                        let _ = init_tx.send(false);
                        return;
                    }

                    use std::os::fd::{FromRawFd, IntoRawFd};
                    let Ok(file) = std::fs::File::open(&path_string) else {
                        rt_core::android_log::log_android(
                            5,
                            &format!(
                                "[RustTracker Video] Failed to open local file {}",
                                path_string
                            ),
                        );
                        AMediaExtractor_delete(extractor);
                        let _ = init_tx.send(false);
                        return;
                    };
                    let file_len = file.metadata().map(|m| m.len() as i64).unwrap_or(0);
                    let raw_fd = file.into_raw_fd();
                    let status = AMediaExtractor_setDataSourceFd(extractor, raw_fd, 0, file_len);
                    if status != 0 {
                        rt_core::android_log::log_android(
                            4,
                            &format!(
                                "[RustTracker Video] AMediaExtractor rejected {} (status {}), falling back to FFmpeg",
                                path_string, status
                            ),
                        );
                        let _ = std::fs::File::from_raw_fd(raw_fd);
                        AMediaExtractor_delete(extractor);
                        let _ = init_tx.send(false);
                        return;
                    }

                    let track_count = AMediaExtractor_getTrackCount(extractor);
                    let mut video_track_idx = None;
                    let mut codec = ptr::null_mut();
                    let mut snap_opt: Option<FormatSnapshot> = None;
                    let mut detected_mime = String::new();

                    let mime_key = c"mime";
                    let color_format_key = c"color-format";

                    for i in 0..track_count {
                        let format = AMediaExtractor_getTrackFormat(extractor, i);
                        if format.is_null() {
                            continue;
                        }

                        let mut mime_ptr: *const c_char = ptr::null();
                        if AMediaFormat_getString(format, mime_key.as_ptr(), &mut mime_ptr)
                            && !mime_ptr.is_null()
                        {
                            let mime_cstr = CStr::from_ptr(mime_ptr);
                            if let Ok(mime_str) = mime_cstr.to_str()
                                && mime_str.starts_with("video/")
                            {
                                let initial_snap = read_format_snapshot(format, None);
                                let profile = get_opt_int32(format, c"profile");
                                let bits_per_sample = get_opt_int32(format, c"bits-per-sample");
                                let candidate_formats = preferred_android_color_formats(
                                    mime_str,
                                    profile,
                                    bits_per_sample,
                                    initial_snap.color_space,
                                    initial_snap.color_trc,
                                );
                                for req_fmt in candidate_formats {
                                    AMediaFormat_setInt32(
                                        format,
                                        color_format_key.as_ptr(),
                                        req_fmt,
                                    );
                                    let candidate = AMediaCodec_createDecoderByType(mime_ptr);
                                    if !candidate.is_null() {
                                        let cfg_status = AMediaCodec_configure(
                                            candidate,
                                            format,
                                            ptr::null_mut(),
                                            ptr::null_mut(),
                                            0,
                                        );
                                        if cfg_status == 0 && AMediaCodec_start(candidate) == 0 {
                                            codec = candidate;
                                            video_track_idx = Some(i);
                                            detected_mime = mime_str.to_string();
                                            snap_opt = Some(initial_snap);
                                            AMediaExtractor_selectTrack(extractor, i);
                                            break;
                                        } else {
                                            AMediaCodec_delete(candidate);
                                        }
                                    }
                                }
                                if !codec.is_null() {
                                    AMediaFormat_delete(format);
                                    break;
                                }
                            }
                        }
                        AMediaFormat_delete(format);
                    }

                    let Some(mut snap) = snap_opt else {
                        rt_core::android_log::log_android(
                            4,
                            &format!(
                                "[RustTracker Video] No suitable MediaCodec video decoder for {}, falling back to FFmpeg",
                                path_string
                            ),
                        );
                        AMediaExtractor_delete(extractor);
                        let _ = std::fs::File::from_raw_fd(raw_fd);
                        let _ = init_tx.send(false);
                        return;
                    };
                    if video_track_idx.is_none() || codec.is_null() {
                        AMediaExtractor_delete(extractor);
                        let _ = std::fs::File::from_raw_fd(raw_fd);
                        let _ = init_tx.send(false);
                        return;
                    }

                    // Query initial output format from started codec if immediately available
                    let init_out_fmt = AMediaCodec_getOutputFormat(codec);
                    if !init_out_fmt.is_null() {
                        snap = read_format_snapshot(init_out_fmt, Some(snap));
                        AMediaFormat_delete(init_out_fmt);
                    }

                    let (video_frame_tx, video_frame_rx) = bounded::<VideoFrame>(16);
                    let (free_video_frame_tx, free_video_frame_rx) = unbounded::<VideoFrame>();
                    for _ in 0..16 {
                        let _ = free_video_frame_tx.try_send(VideoFrame::empty(0));
                    }

                    let codec_label = rt_core::state::video_mime_codec_label(&detected_mime);
                    if let Ok(mut state) = shared_state.lock() {
                        state.video_frame_rx = Some(video_frame_rx);
                        state.free_video_frame_tx = Some(free_video_frame_tx);
                        state.has_video_stream = true;
                        state.mobile_hud_tab = rt_core::state::MobileHudTab::Video;
                        state.stats.video_hw_decoder =
                            android_hw_decoder_label(snap.geom.layout).to_string();
                        let (disp_w, disp_h) = if snap.rotation == 90 || snap.rotation == 270 {
                            (snap.geom.active_h, snap.geom.active_w)
                        } else {
                            (snap.geom.active_w, snap.geom.active_h)
                        };
                        state.video_info = Some(format!(
                            "Video Stream: {}x{} {} (Hardware)",
                            disp_w, disp_h, codec_label
                        ));
                    }

                    rt_core::android_log::log_android(
                        3,
                        &format!(
                            "[RustTracker Video] MediaCodec started ({} active={}x{} coded={}x{} rot={}deg layout={:?})",
                            detected_mime,
                            snap.geom.active_w,
                            snap.geom.active_h,
                            snap.geom.coded_w,
                            snap.geom.coded_h,
                            snap.rotation,
                            snap.geom.layout
                        ),
                    );
                    let _ = init_tx.send(true);

                    let mut local_epoch = 0;
                    let mut eos_input = false;
                    let mut is_first_frame_after_seek = true;
                    let mut frame_generation: u64 = 0;

                    while !stop_token.load(Ordering::Relaxed) {
                        if let Ok(state) = shared_state.lock()
                            && state.seek_epoch > local_epoch
                        {
                            let seek_us = (state.current_seconds * 1_000_000.0) as i64;
                            AMediaExtractor_seekTo(
                                extractor,
                                seek_us,
                                AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC,
                            );
                            AMediaCodec_flush(codec);
                            local_epoch = state.seek_epoch;
                            eos_input = false;
                            is_first_frame_after_seek = true;
                        }

                        if !eos_input {
                            let in_idx = AMediaCodec_dequeueInputBuffer(codec, 2000);
                            if in_idx >= 0 {
                                let mut buf_cap: usize = 0;
                                let in_buf =
                                    AMediaCodec_getInputBuffer(codec, in_idx as usize, &mut buf_cap);
                                if !in_buf.is_null() && buf_cap > 0 {
                                    let sample_size =
                                        AMediaExtractor_readSampleData(extractor, in_buf, buf_cap);
                                    if sample_size > 0 {
                                        let sample_pts =
                                            AMediaExtractor_getSampleTime(extractor).max(0) as u64;
                                        let sample_flags =
                                            AMediaExtractor_getSampleFlags(extractor);
                                        AMediaCodec_queueInputBuffer(
                                            codec,
                                            in_idx as usize,
                                            0,
                                            sample_size as usize,
                                            sample_pts,
                                            sample_flags,
                                        );
                                        AMediaExtractor_advance(extractor);
                                    } else {
                                        AMediaCodec_queueInputBuffer(
                                            codec,
                                            in_idx as usize,
                                            0,
                                            0,
                                            0,
                                            AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM,
                                        );
                                        eos_input = true;
                                    }
                                }
                            }
                        }

                        let mut info = AMediaCodecBufferInfo {
                            offset: 0,
                            size: 0,
                            presentation_time_us: 0,
                            flags: 0,
                        };
                        let out_idx = AMediaCodec_dequeueOutputBuffer(codec, &mut info, 5000);

                        if out_idx >= 0 {
                            let pts_sec = (info.presentation_time_us.max(0) as f64) / 1_000_000.0;

                            let mut skip_push = false;
                            if is_first_frame_after_seek {
                                let (cached_seconds, current_epoch) = shared_state
                                    .lock()
                                    .map(|s| (s.current_seconds, s.seek_epoch))
                                    .unwrap_or((pts_sec, local_epoch));
                                if current_epoch > local_epoch || pts_sec < cached_seconds - 0.15 {
                                    skip_push = true;
                                } else {
                                    is_first_frame_after_seek = false;
                                }
                            } else {
                                loop {
                                    if stop_token.load(Ordering::Relaxed) {
                                        AMediaCodec_releaseOutputBuffer(
                                            codec,
                                            out_idx as usize,
                                            false,
                                        );
                                        AMediaCodec_stop(codec);
                                        AMediaCodec_delete(codec);
                                        AMediaExtractor_delete(extractor);
                                        let _ = std::fs::File::from_raw_fd(raw_fd);
                                        return;
                                    }
                                    let (current_audio_pts, current_epoch, is_paused, track_ended) =
                                        shared_state
                                            .lock()
                                            .map(|s| {
                                                (
                                                    s.current_seconds,
                                                    s.seek_epoch,
                                                    s.is_paused,
                                                    s.track_ended,
                                                )
                                            })
                                            .unwrap_or((pts_sec, local_epoch, false, false));
                                    if track_ended || current_epoch > local_epoch {
                                        skip_push = true;
                                        break;
                                    }
                                    if pts_sec < current_audio_pts - 0.15 {
                                        skip_push = true;
                                        break;
                                    }
                                    if !is_paused && pts_sec <= current_audio_pts + 0.05 {
                                        break;
                                    }
                                    if pts_sec > current_audio_pts + 30.0 {
                                        skip_push = true;
                                        break;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(
                                        if is_paused { 25 } else { 4 },
                                    ));
                                }
                            }

                            if info.size > 0 && !skip_push {
                                let mut out_size: usize = 0;
                                let out_buf = AMediaCodec_getOutputBuffer(
                                    codec,
                                    out_idx as usize,
                                    &mut out_size,
                                );
                                let offset = info.offset.max(0) as usize;
                                let frame_len = info.size as usize;
                                if !out_buf.is_null() && out_size >= offset + frame_len {
                                    let src_slice =
                                        std::slice::from_raw_parts(out_buf.add(offset), frame_len);

                                    let mut frame = free_video_frame_rx
                                        .try_recv()
                                        .unwrap_or_else(|_| VideoFrame::empty(snap.rotation));

                                    frame_generation = frame_generation.wrapping_add(1);
                                    frame.pts = pts_sec;
                                    frame.rotation = snap.rotation;
                                    frame.generation = frame_generation;
                                    frame.color_space = snap.color_space;
                                    frame.color_range = snap.color_range;
                                    frame.color_trc = snap.color_trc;

                                    unpack_android_yuv_frame(src_slice, &snap.geom, &mut frame);

                                    let _ = video_frame_tx.try_send(frame);
                                }
                            }

                            if info.flags & AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM != 0 {
                                eos_input = true;
                            }

                            AMediaCodec_releaseOutputBuffer(codec, out_idx as usize, false);
                        } else if out_idx == AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED {
                            let out_format = AMediaCodec_getOutputFormat(codec);
                            if !out_format.is_null() {
                                snap = read_format_snapshot(out_format, Some(snap));
                                AMediaFormat_delete(out_format);

                                if let Ok(mut state) = shared_state.lock() {
                                    state.stats.video_hw_decoder =
                                        android_hw_decoder_label(snap.geom.layout).to_string();
                                    let (disp_w, disp_h) =
                                        if snap.rotation == 90 || snap.rotation == 270 {
                                            (snap.geom.active_h, snap.geom.active_w)
                                        } else {
                                            (snap.geom.active_w, snap.geom.active_h)
                                        };
                                    state.video_info = Some(format!(
                                        "Video Stream: {}x{} {} (Hardware)",
                                        disp_w, disp_h, codec_label
                                    ));
                                }

                                rt_core::android_log::log_android(
                                    3,
                                    &format!(
                                        "[RustTracker Video] Output format changed: active={}x{} coded={}x{} crop=({},{}) stride={} slice_h={} rot={}deg layout={:?} cs={} cr={} trc={}",
                                        snap.geom.active_w,
                                        snap.geom.active_h,
                                        snap.geom.coded_w,
                                        snap.geom.coded_h,
                                        snap.geom.crop_left,
                                        snap.geom.crop_top,
                                        snap.geom.stride_bytes,
                                        snap.geom.slice_height,
                                        snap.rotation,
                                        snap.geom.layout,
                                        snap.color_space,
                                        snap.color_range,
                                        snap.color_trc
                                    ),
                                );
                            }
                        } else if out_idx == AMEDIACODEC_INFO_TRY_AGAIN_LATER {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                    }

                    AMediaCodec_stop(codec);
                    AMediaCodec_delete(codec);
                    AMediaExtractor_delete(extractor);
                    let _ = std::fs::File::from_raw_fd(raw_fd);
                    rt_core::android_log::log_android(3, "[RustTracker Video] Decoder cleanly shut down");
                }));
                if res.is_err() {
                    let _ = init_tx.send(false);
                }
            })
            .is_ok();

        if !spawn_ok {
            return false;
        }
        init_rx.recv().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rt_core::state::{VideoChromaLayout, VideoFrame, WGPU_ROW_PITCH_ALIGNMENT};

    fn empty_frame() -> VideoFrame {
        VideoFrame::empty(0)
    }

    #[test]
    fn test_android_crop_stride_and_color_metadata_mapping() {
        // 1080p stream padded to 1920x1088 macroblocks with 2048-byte row stride
        let geom =
            resolve_android_buffer_geometry(1920, 1088, Some((0, 0, 1919, 1079)), 2048, 1088, 21);
        assert_eq!(geom.active_w, 1920);
        assert_eq!(geom.active_h, 1080);
        assert_eq!(geom.stride_bytes, 2048);
        assert_eq!(geom.slice_height, 1088);
        assert_eq!(geom.layout, AndroidYuvLayout::SemiPlanarNv12);
        assert_eq!(
            android_hw_decoder_label(geom.layout),
            "HW (MediaCodec -> 2-Plane NV12)"
        );

        // HDR10 BT.2020 + Limited Range + ST.2084 PQ
        assert_eq!(map_android_color_standard(Some(6), 1080), 9);
        assert_eq!(map_android_color_range(Some(2)), 1);
        assert_eq!(map_android_color_range(Some(1)), 2);
        assert_eq!(map_android_color_transfer(Some(6)), 16);
        assert_eq!(map_android_color_transfer(Some(7)), 18);
    }

    #[test]
    fn test_unpack_nv12_and_i420_with_crop_and_stride() {
        // Coded 8x8 buffer with stride=16, slice_height=8, cropped to inner (left=2, top=2, right=5, bottom=5) -> 4x4 active
        let stride = 16usize;
        let slice_h = 8usize;
        let mut raw_nv12 = vec![0u8; stride * slice_h + stride * (slice_h / 2)];
        // Populate active Y rows 2..6, cols 2..6 with distinct marker values
        for r in 0..4 {
            for c in 0..4 {
                raw_nv12[(2 + r) * stride + (2 + c)] = (10 + r * 4 + c) as u8;
            }
        }
        // Populate active UV rows 1..3, byte cols 2..6 (2 chroma pairs per row)
        let uv_base = stride * slice_h;
        for r in 0..2 {
            for c in 0..2 {
                let idx = uv_base + (1 + r) * stride + 2 + c * 2;
                raw_nv12[idx] = (100 + r * 2 + c) as u8;
                raw_nv12[idx + 1] = (200 + r * 2 + c) as u8;
            }
        }

        let geom_nv12 = resolve_android_buffer_geometry(8, 8, Some((2, 2, 5, 5)), 16, 8, 21);
        let mut frame = empty_frame();
        unpack_android_yuv_frame(&raw_nv12, &geom_nv12, &mut frame);
        assert_eq!((frame.width, frame.height), (4, 4));
        assert_eq!(frame.chroma_layout, VideoChromaLayout::SemiPlanar420);
        assert_eq!(frame.y_stride, WGPU_ROW_PITCH_ALIGNMENT);
        assert_eq!(frame.u_stride, WGPU_ROW_PITCH_ALIGNMENT);
        assert_eq!(frame.v_stride, 0);
        assert!(frame.v_plane.is_empty());
        assert_eq!(frame.y_plane[0], 10);
        assert_eq!(frame.y_plane[3 * frame.y_stride + 3], 25);
        // Row 0 interleaved [U0, V0, U1, V1], Row 1 interleaved [U2, V2, U3, V3]
        assert_eq!(&frame.u_plane[0..4], &[100, 200, 101, 201]);
        assert_eq!(
            &frame.u_plane[frame.u_stride..frame.u_stride + 4],
            &[102, 202, 103, 203]
        );

        // Planar I420 (color-format = 19) with the same 8x8 -> 4x4 crop & stride=16
        let c_stride = stride / 2;
        let c_slice = slice_h / 2;
        let mut raw_i420 = vec![0u8; stride * slice_h + 2 * c_stride * c_slice];
        for r in 0..4 {
            for c in 0..4 {
                raw_i420[(2 + r) * stride + (2 + c)] = (30 + r * 4 + c) as u8;
            }
        }
        let u_base = stride * slice_h;
        let v_base = u_base + c_stride * c_slice;
        for r in 0..2 {
            for c in 0..2 {
                raw_i420[u_base + (1 + r) * c_stride + (1 + c)] = (110 + r * 2 + c) as u8;
                raw_i420[v_base + (1 + r) * c_stride + (1 + c)] = (210 + r * 2 + c) as u8;
            }
        }
        let geom_i420 = resolve_android_buffer_geometry(8, 8, Some((2, 2, 5, 5)), 16, 8, 19);
        unpack_android_yuv_frame(&raw_i420, &geom_i420, &mut frame);
        assert_eq!(frame.chroma_layout, VideoChromaLayout::Planar420);
        assert_eq!(frame.y_plane[0], 30);
        assert_eq!(&frame.u_plane[0..2], &[110, 111]);
        assert_eq!(
            &frame.u_plane[frame.u_stride..frame.u_stride + 2],
            &[112, 113]
        );
        assert_eq!(&frame.v_plane[0..2], &[210, 211]);
        assert_eq!(
            &frame.v_plane[frame.v_stride..frame.v_stride + 2],
            &[212, 213]
        );
    }

    #[test]
    fn test_preferred_android_color_formats_and_p010_unpack() {
        // 10-bit HEVC Main10 / HDR10 PQ negotiates P010 (54) first
        assert_eq!(
            preferred_android_color_formats("video/hevc", Some(2), None, 1, 1),
            [54, 21, 19]
        );
        assert_eq!(
            preferred_android_color_formats("video/av01", None, None, 9, 16),
            [54, 21, 19]
        );
        // 8-bit SDR H.264 negotiates NV12 (21) -> I420 (19) -> P010 (54)
        assert_eq!(
            preferred_android_color_formats("video/avc", Some(8), Some(8), 1, 1),
            [21, 19, 54]
        );

        // Coded 8x8 P010LE buffer (stride = 32 bytes = 16 u16 samples, slice_height = 8),
        // odd crop rect (left=3, top=3, right=6, bottom=6) -> 2x2-aligned origin (2, 2), active 4x4.
        let stride_bytes = 32usize;
        let slice_h = 8usize;
        let mut raw_p010 = vec![0u8; stride_bytes * slice_h + stride_bytes * (slice_h / 2)];
        for r in 0..4 {
            for c in 0..4 {
                let sample: u16 = ((512 + r * 16 + c) as u16) << 6;
                let off = (2 + r) * stride_bytes + (2 + c) * 2;
                raw_p010[off..off + 2].copy_from_slice(&sample.to_le_bytes());
            }
        }
        let uv_base = stride_bytes * slice_h;
        for r in 0..2 {
            for c in 0..2 {
                let u_val: u16 = ((400 + r * 4 + c) as u16) << 6;
                let v_val: u16 = ((700 + r * 4 + c) as u16) << 6;
                let off = uv_base + (1 + r) * stride_bytes + 4 + c * 4;
                raw_p010[off..off + 2].copy_from_slice(&u_val.to_le_bytes());
                raw_p010[off + 2..off + 4].copy_from_slice(&v_val.to_le_bytes());
            }
        }

        let geom_p010 = resolve_android_buffer_geometry(8, 8, Some((3, 3, 6, 6)), 32, 8, 54);
        assert_eq!(geom_p010.layout, AndroidYuvLayout::SemiPlanarP010);
        assert_eq!(
            android_hw_decoder_label(geom_p010.layout),
            "HW (MediaCodec -> 2-Plane P010)"
        );
        assert_eq!((geom_p010.crop_left, geom_p010.crop_top), (2, 2));
        assert_eq!((geom_p010.active_w, geom_p010.active_h), (4, 4));

        let mut frame = empty_frame();
        unpack_android_yuv_frame(&raw_p010, &geom_p010, &mut frame);
        assert_eq!(frame.bit_depth, 16);
        assert_eq!(frame.chroma_layout, VideoChromaLayout::SemiPlanar420);
        assert_eq!(frame.y_stride, WGPU_ROW_PITCH_ALIGNMENT);
        assert_eq!(frame.u_stride, WGPU_ROW_PITCH_ALIGNMENT);
        assert_eq!(frame.v_stride, 0);
        assert!(frame.v_plane.is_empty());
        assert_eq!(
            u16::from_le_bytes([frame.y_plane[0], frame.y_plane[1]]),
            512u16 << 6
        );
        assert_eq!(
            u16::from_le_bytes([frame.u_plane[0], frame.u_plane[1]]),
            400u16 << 6
        );
        assert_eq!(
            u16::from_le_bytes([frame.u_plane[2], frame.u_plane[3]]),
            700u16 << 6
        );
        let row1 = frame.u_stride;
        assert_eq!(
            u16::from_le_bytes([frame.u_plane[row1 + 4], frame.u_plane[row1 + 5]]),
            405u16 << 6
        );
        assert_eq!(
            u16::from_le_bytes([frame.u_plane[row1 + 6], frame.u_plane[row1 + 7]]),
            705u16 << 6
        );

        // Truncated chroma plane fills neutral 10-bit midpoint 0x8000 ([0x00, 0x80] LE) instead of 0x0000 green
        unpack_android_yuv_frame(&raw_p010[..uv_base], &geom_p010, &mut frame);
        assert_eq!(
            &frame.u_plane[0..8],
            &[0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80]
        );
        assert_eq!(
            &frame.u_plane[row1..row1 + 8],
            &[0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80]
        );
        assert!(frame.v_plane.is_empty());

        // Vendor P010 reporting padded pixel stride (e.g. fmt_stride = 12 samples for coded_w = 8 -> 24 bytes)
        let geom_p010_px_stride = resolve_android_buffer_geometry(8, 8, None, 12, 8, 54);
        assert_eq!(geom_p010_px_stride.stride_bytes, 24);
    }

    #[test]
    fn test_android_nv21_and_yv12_16byte_chroma_stride_unpack() {
        let mut frame = empty_frame();

        // 1. NV21 (color-format = 17) keeps interleaved VU pairs in u_plane as SemiPlanarVu420
        let stride = 16usize;
        let slice_h = 8usize;
        let mut raw_nv21 = vec![0u8; stride * slice_h + stride * (slice_h / 2)];
        raw_nv21[0..4].copy_from_slice(&[50, 51, 52, 53]);
        let vu_base = stride * slice_h;
        raw_nv21[vu_base..vu_base + 4].copy_from_slice(&[210, 90, 211, 91]); // [V0, U0, V1, U1]
        let geom_nv21 = resolve_android_buffer_geometry(4, 4, None, 16, 8, 17);
        assert_eq!(geom_nv21.layout, AndroidYuvLayout::SemiPlanarNv21);
        assert_eq!(
            android_hw_decoder_label(geom_nv21.layout),
            "HW (MediaCodec -> 2-Plane NV21)"
        );
        unpack_android_yuv_frame(&raw_nv21, &geom_nv21, &mut frame);
        assert_eq!(frame.chroma_layout, VideoChromaLayout::SemiPlanarVu420);
        assert!(frame.v_plane.is_empty());
        assert_eq!(&frame.u_plane[0..4], &[210, 90, 211, 91]);

        // 2. YV12 (HAL_PIXEL_FORMAT_YV12 = 0x32315659): luma stride = 20 -> c_stride = align16(10) = 16, planes Y -> V -> U
        let y_stride = 20usize;
        let c_stride = 16usize;
        let mut raw_yv12 = vec![0u8; y_stride * 4 + 2 * c_stride * 2];
        raw_yv12[0..4].copy_from_slice(&[60, 61, 62, 63]);
        let v_plane_off = y_stride * 4;
        let u_plane_off = v_plane_off + c_stride * 2;
        raw_yv12[v_plane_off..v_plane_off + 2].copy_from_slice(&[222, 223]);
        raw_yv12[v_plane_off + c_stride..v_plane_off + c_stride + 2].copy_from_slice(&[224, 225]);
        raw_yv12[u_plane_off..u_plane_off + 2].copy_from_slice(&[88, 89]);
        raw_yv12[u_plane_off + c_stride..u_plane_off + c_stride + 2].copy_from_slice(&[90, 91]);

        let geom_yv12 = resolve_android_buffer_geometry(4, 4, None, 20, 4, 0x3231_5659);
        assert_eq!(geom_yv12.layout, AndroidYuvLayout::PlanarYv12);
        assert_eq!(
            android_hw_decoder_label(geom_yv12.layout),
            "HW (MediaCodec -> 3-Plane YV12)"
        );
        unpack_android_yuv_frame(&raw_yv12, &geom_yv12, &mut frame);
        assert_eq!(frame.chroma_layout, VideoChromaLayout::Planar420);
        assert_eq!(&frame.u_plane[0..2], &[88, 89]);
        assert_eq!(
            &frame.u_plane[frame.u_stride..frame.u_stride + 2],
            &[90, 91]
        );
        assert_eq!(&frame.v_plane[0..2], &[222, 223]);
        assert_eq!(
            &frame.v_plane[frame.v_stride..frame.v_stride + 2],
            &[224, 225]
        );
    }
}
