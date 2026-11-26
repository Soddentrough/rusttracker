#![allow(dead_code)]

use ffmpeg_next as ffmpeg;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempTestDir(PathBuf);

impl TempTestDir {
    pub fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("rt_int_test_{}_{}_{}", tag, std::process::id(), id));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn build_wav_bytes(sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
    let bits_per_sample = 16u16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * block_align as u32;
    let data_size = (frames * block_align as usize) as u32;
    let riff_size = 36 + data_size;

    let mut buf = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());

    for f in 0..frames {
        let t = f as f32 / sample_rate as f32;
        for c in 0..channels {
            let freq = 220.0 * (1.0 + c as f32 * 0.5);
            let sample = ((t * freq * std::f32::consts::TAU).sin() * 16000.0) as i16;
            buf.extend_from_slice(&sample.to_le_bytes());
        }
    }
    buf
}

/// Builds a self-contained Standard MIDI File (Format 0, 480 PPQN, 128 BPM, ~30s duration)
/// with repeating C-major quarter notes.
pub fn build_minimal_midi_bytes() -> Vec<u8> {
    let mut track = Vec::new();
    // Tempo meta-event: 468_750 us/quarter = 128 BPM (0x07270E)
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0x27, 0x0E]);
    // Program change channel 0 -> Acoustic Grand Piano (0)
    track.extend_from_slice(&[0x00, 0xC0, 0x00]);

    let notes = [60u8, 64, 67, 72];
    for step in 0..64usize {
        let note = notes[step % notes.len()];
        // Delta 0 -> Note On
        track.extend_from_slice(&[0x00, 0x90, note, 100]);
        // Delta 480 (VLQ 0x83, 0x60) -> Note Off
        track.extend_from_slice(&[0x83, 0x60, 0x80, note, 0]);
    }
    // End of track
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);

    let mut smf = Vec::with_capacity(14 + 8 + track.len());
    smf.extend_from_slice(b"MThd");
    smf.extend_from_slice(&6u32.to_be_bytes());
    smf.extend_from_slice(&0u16.to_be_bytes()); // Format 0
    smf.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    smf.extend_from_slice(&480u16.to_be_bytes()); // 480 PPQN
    smf.extend_from_slice(b"MTrk");
    smf.extend_from_slice(&(track.len() as u32).to_be_bytes());
    smf.extend_from_slice(&track);
    smf
}

struct AudioTrackSpec {
    codec_id: ffmpeg::codec::Id,
    channels: u16,
    sample_rate: u32,
    base_freq: f32,
    bit_rate: usize,
}

fn encode_container_with_streams(
    path: &Path,
    audio_specs: &[AudioTrackSpec],
    duration_secs: f32,
    video_rotation_deg: Option<i32>,
) -> anyhow::Result<()> {
    ffmpeg::init()?;

    let mut octx = ffmpeg::format::output(path)?;
    let global_header = octx
        .format()
        .flags()
        .contains(ffmpeg::format::flag::Flags::GLOBAL_HEADER);

    struct OpenedAudio {
        stream_idx: usize,
        enc: ffmpeg::encoder::Audio,
        resampler: ffmpeg::software::resampling::Context,
        frame_size: usize,
        channels: usize,
        sample_rate: u32,
        base_freq: f32,
        enc_tb: ffmpeg::Rational,
        out_tb: ffmpeg::Rational,
    }

    let mut audio_encoders = Vec::with_capacity(audio_specs.len());
    for spec in audio_specs {
        let codec = ffmpeg::encoder::find(spec.codec_id)
            .ok_or_else(|| anyhow::anyhow!("FFmpeg encoder {:?} not found", spec.codec_id))?;
        let mut ost = octx.add_stream(codec)?;
        let stream_idx = ost.index();

        let mut ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
        if global_header {
            unsafe {
                (*ctx.as_mut_ptr()).flags |= ffmpeg::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        let mut enc_cfg = ctx.encoder().audio()?;
        let layout = ffmpeg::channel_layout::ChannelLayout::default(spec.channels as i32);
        enc_cfg.set_channel_layout(layout);
        enc_cfg.set_rate(spec.sample_rate as i32);
        enc_cfg.set_bit_rate(spec.bit_rate);
        enc_cfg.set_time_base(ffmpeg::Rational::new(1, spec.sample_rate as i32));

        let target_fmt = codec
            .audio()
            .ok()
            .and_then(|a| a.formats())
            .and_then(|mut fmts| fmts.next())
            .unwrap_or(ffmpeg::format::Sample::F32(
                ffmpeg::format::sample::Type::Planar,
            ));
        enc_cfg.set_format(target_fmt);

        let opened = enc_cfg.open()?;
        ost.set_parameters(&opened);
        ost.set_time_base(ffmpeg::Rational::new(1, spec.sample_rate as i32));

        let frame_size = (opened.frame_size() as usize).max(1024);
        let resampler = ffmpeg::software::resampling::Context::get(
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
            layout,
            spec.sample_rate,
            target_fmt,
            layout,
            spec.sample_rate,
        )?;

        audio_encoders.push(OpenedAudio {
            stream_idx,
            enc_tb: opened.time_base(),
            out_tb: ffmpeg::Rational::new(1, spec.sample_rate as i32),
            enc: opened,
            resampler,
            frame_size,
            channels: spec.channels as usize,
            sample_rate: spec.sample_rate,
            base_freq: spec.base_freq,
        });
    }

    struct OpenedVideo {
        stream_idx: usize,
        enc: ffmpeg::encoder::Video,
        enc_tb: ffmpeg::Rational,
        out_tb: ffmpeg::Rational,
        width: u32,
        height: u32,
    }

    let mut video_encoder: Option<OpenedVideo> = None;
    if let Some(rot_deg) = video_rotation_deg {
        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::MPEG4)
            .ok_or_else(|| anyhow::anyhow!("FFmpeg MPEG4 video encoder not found"))?;
        let mut ost = octx.add_stream(codec)?;
        let stream_idx = ost.index();

        let mut ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
        if global_header {
            unsafe {
                (*ctx.as_mut_ptr()).flags |= ffmpeg::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        let mut v_cfg = ctx.encoder().video()?;
        v_cfg.set_width(640);
        v_cfg.set_height(360);
        v_cfg.set_format(ffmpeg::format::Pixel::YUV420P);
        v_cfg.set_time_base(ffmpeg::Rational::new(1, 30));
        v_cfg.set_frame_rate(Some(ffmpeg::Rational::new(30, 1)));
        v_cfg.set_bit_rate(400_000);
        v_cfg.set_gop(15);
        v_cfg.set_max_b_frames(0);

        let opened = v_cfg.open()?;
        ost.set_parameters(&opened);
        ost.set_time_base(ffmpeg::Rational::new(1, 30));

        if rot_deg != 0 {
            let mut meta = ffmpeg::Dictionary::new();
            meta.set("rotate", &rot_deg.to_string());
            ost.set_metadata(meta);

            unsafe {
                let par = (*ost.as_mut_ptr()).codecpar;
                let sd = ffmpeg::ffi::av_packet_side_data_new(
                    &mut (*par).coded_side_data,
                    &mut (*par).nb_coded_side_data,
                    ffmpeg::ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
                    36,
                    0,
                );
                if !sd.is_null() && !(*sd).data.is_null() {
                    ffmpeg::ffi::av_display_rotation_set(
                        (*sd).data.cast::<i32>(),
                        -(rot_deg as f64),
                    );
                }
            }
        }

        video_encoder = Some(OpenedVideo {
            stream_idx,
            enc_tb: opened.time_base(),
            out_tb: ffmpeg::Rational::new(1, 30),
            enc: opened,
            width: 640,
            height: 360,
        });
    }

    octx.write_header()?;
    // Update stream time_bases after write_header (muxers like MP4/MKV may adjust them)
    for ae in &mut audio_encoders {
        ae.out_tb = octx.stream(ae.stream_idx).unwrap().time_base();
    }
    if let Some(ref mut ve) = video_encoder {
        ve.out_tb = octx.stream(ve.stream_idx).unwrap().time_base();
    }

    for ae in &mut audio_encoders {
        let total_samples = (duration_secs * ae.sample_rate as f32).round() as usize;
        let layout = ffmpeg::channel_layout::ChannelLayout::default(ae.channels as i32);
        let mut pts = 0i64;
        while (pts as usize) < total_samples {
            let n_samples = ae.frame_size;
            let mut in_frame = ffmpeg::frame::Audio::new(
                ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
                n_samples,
                layout,
            );
            in_frame.set_rate(ae.sample_rate);
            in_frame.set_pts(Some(pts));
            for c in 0..ae.channels {
                let freq = ae.base_freq + c as f32 * 55.0;
                let plane = in_frame.plane_mut::<f32>(c);
                for (i, dst) in plane.iter_mut().enumerate() {
                    let t = (pts as usize + i) as f32 / ae.sample_rate as f32;
                    *dst = 0.25 * (t * freq * std::f32::consts::TAU).sin();
                }
            }

            let mut conv = ffmpeg::frame::Audio::empty();
            ae.resampler.run(&in_frame, &mut conv)?;
            conv.set_pts(Some(pts));
            ae.enc.send_frame(&conv)?;

            let mut pkt = ffmpeg::Packet::empty();
            while ae.enc.receive_packet(&mut pkt).is_ok() {
                pkt.set_stream(ae.stream_idx);
                pkt.rescale_ts(ae.enc_tb, ae.out_tb);
                pkt.write_interleaved(&mut octx)?;
            }
            pts += n_samples as i64;
        }
        ae.enc.send_eof()?;
        let mut pkt = ffmpeg::Packet::empty();
        while ae.enc.receive_packet(&mut pkt).is_ok() {
            pkt.set_stream(ae.stream_idx);
            pkt.rescale_ts(ae.enc_tb, ae.out_tb);
            pkt.write_interleaved(&mut octx)?;
        }
    }

    if let Some(ref mut ve) = video_encoder {
        let total_frames = (duration_secs * 30.0).round() as i64;
        for f_idx in 0..total_frames {
            let mut vf =
                ffmpeg::frame::Video::new(ffmpeg::format::Pixel::YUV420P, ve.width, ve.height);
            vf.set_pts(Some(f_idx));
            let y_val = ((f_idx * 7) % 180 + 32) as u8;
            vf.data_mut(0).fill(y_val);
            vf.data_mut(1).fill(0x80);
            vf.data_mut(2).fill(0x80);
            ve.enc.send_frame(&vf)?;
            let mut pkt = ffmpeg::Packet::empty();
            while ve.enc.receive_packet(&mut pkt).is_ok() {
                pkt.set_stream(ve.stream_idx);
                pkt.rescale_ts(ve.enc_tb, ve.out_tb);
                pkt.write_interleaved(&mut octx)?;
            }
        }
        ve.enc.send_eof()?;
        let mut pkt = ffmpeg::Packet::empty();
        while ve.enc.receive_packet(&mut pkt).is_ok() {
            pkt.set_stream(ve.stream_idx);
            pkt.rescale_ts(ve.enc_tb, ve.out_tb);
            pkt.write_interleaved(&mut octx)?;
        }
    }

    octx.write_trailer()?;
    Ok(())
}

pub fn encode_synthetic_flac(
    path: &Path,
    channels: u16,
    sample_rate: u32,
    duration_secs: f32,
) -> anyhow::Result<()> {
    encode_container_with_streams(
        path,
        &[AudioTrackSpec {
            codec_id: ffmpeg::codec::Id::FLAC,
            channels,
            sample_rate,
            base_freq: 220.0,
            bit_rate: 0,
        }],
        duration_secs,
        None,
    )
}

pub fn encode_synthetic_aac_51_media(
    path: &Path,
    duration_secs: f32,
    video_rotation_deg: Option<i32>,
) -> anyhow::Result<()> {
    encode_container_with_streams(
        path,
        &[AudioTrackSpec {
            codec_id: ffmpeg::codec::Id::AAC,
            channels: 6,
            sample_rate: 48_000,
            base_freq: 220.0,
            bit_rate: 256_000,
        }],
        duration_secs,
        video_rotation_deg,
    )
}

pub fn encode_synthetic_multitrack_mkv(path: &Path, duration_secs: f32) -> anyhow::Result<()> {
    encode_container_with_streams(
        path,
        &[
            AudioTrackSpec {
                codec_id: ffmpeg::codec::Id::AC3,
                channels: 6,
                sample_rate: 48_000,
                base_freq: 220.0,
                bit_rate: 384_000,
            },
            AudioTrackSpec {
                codec_id: ffmpeg::codec::Id::EAC3,
                channels: 6,
                sample_rate: 48_000,
                base_freq: 330.0,
                bit_rate: 448_000,
            },
            AudioTrackSpec {
                codec_id: ffmpeg::codec::Id::AAC,
                channels: 2,
                sample_rate: 48_000,
                base_freq: 440.0,
                bit_rate: 192_000,
            },
        ],
        duration_secs,
        None,
    )
}
