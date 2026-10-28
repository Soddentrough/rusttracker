pub type BitstreamLaunchResult = (
    std::thread::JoinHandle<()>,
    u32,
    u16,
    String,
    bool,
    String,
    f64,
);

#[cfg(target_os = "linux")]
pub fn start_bitstream_thread(
    file_path: &str,
    shared_state: std::sync::Arc<std::sync::Mutex<crate::state::AppState>>,
    tx: crossbeam_channel::Sender<crate::audio::DspMessage>,
    stop_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<BitstreamLaunchResult> {
    use anyhow::Context;

    if shared_state
        .lock()
        .map(|s| s.force_stereo_downmix)
        .unwrap_or(false)
    {
        return Err(anyhow::anyhow!(
            "Force stereo downmix enabled; skipping passthrough"
        ));
    }

    println!("[bitstream] Probing codec via ffmpeg-next on Linux...");
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
    ffmpeg_next::init().context("Failed to initialize ffmpeg-next")?;

    let mut dict = ffmpeg_next::Dictionary::new();
    dict.set("probesize", "5000000");
    dict.set("analyzeduration", "5000000");
    let mut ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
        .context("Failed to open input file")?;

    let best_audio = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
        .ok_or_else(|| anyhow::anyhow!("No audio stream found"))?;

    let codec_id = best_audio.parameters().id();
    let codec_name = match codec_id {
        ffmpeg_next::codec::Id::TRUEHD => "truehd",
        ffmpeg_next::codec::Id::EAC3 => "eac3",
        ffmpeg_next::codec::Id::DTS => "dts",
        ffmpeg_next::codec::Id::AC3 => "ac3",
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported codec for bitstreaming: {:?}",
                codec_id
            ));
        }
    }
    .to_string();

    let meta_duration = if ictx.duration() > 0 {
        ictx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64
    } else {
        0.0
    };
    let meta_artist = {
        let meta = ictx.metadata();
        meta.get("artist")
            .or_else(|| meta.get("ARTIST"))
            .map(|s| s.to_string())
            .unwrap_or_default()
    };

    let video_setup = crate::audio::probe_video_stream(&ictx);
    let has_video = video_setup.is_some();

    let mut audio_bitrate_kbps = crate::audio::probe_stream_bitrate_kbps(&best_audio);
    let parameters = best_audio.parameters();
    let best_audio_index = best_audio.index();

    let probe_ctx = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone())
        .context("Failed to create probe context")?;
    let probe_decoder = probe_ctx
        .decoder()
        .audio()
        .context("Failed to create probe decoder")?;
    let decoder_sample_rate = probe_decoder.rate();
    if audio_bitrate_kbps.is_none() && probe_decoder.bit_rate() > 0 {
        audio_bitrate_kbps = Some((probe_decoder.bit_rate() / 1000).max(1) as u32);
    }

    let video_fps = video_setup.as_ref().map(|vs| vs.fps).unwrap_or(0.0);
    let mut video_bitrate_kbps = video_setup.as_ref().and_then(|vs| vs.bitrate_kbps);
    let container_kbps = if ictx.bit_rate() > 0 {
        Some((ictx.bit_rate() / 1000).max(1) as u32)
    } else {
        None
    };
    if video_bitrate_kbps.is_none()
        && has_video
        && let Some(c_kbps) = container_kbps
    {
        let a_kbps = audio_bitrate_kbps.unwrap_or(0);
        video_bitrate_kbps = Some(if c_kbps > a_kbps {
            c_kbps - a_kbps
        } else {
            c_kbps
        });
    }
    if audio_bitrate_kbps.is_none()
        && let Some(c_kbps) = container_kbps
    {
        if !has_video {
            audio_bitrate_kbps = Some(c_kbps);
        } else if let Some(v_kbps) = video_bitrate_kbps
            && c_kbps > v_kbps
        {
            audio_bitrate_kbps = Some(c_kbps - v_kbps);
        }
    }
    if let Ok(mut state) = shared_state.lock() {
        state.bitrate = audio_bitrate_kbps;
        state.stats.audio_bitrate_kbps = audio_bitrate_kbps;
        state.stats.video_fps = video_fps;
        state.stats.video_bitrate_kbps = video_bitrate_kbps;
    }

    let pw_rate = if codec_name == "truehd" || codec_name == "dts" {
        192000
    } else {
        48000
    };
    println!(
        "[bitstream] Codec: {}, Decoder Rate: {}, Output Rate: {}",
        codec_name, decoder_sample_rate, pw_rate
    );

    let pipe_path = std::env::temp_dir()
        .join(format!("rusttracker_bitstream_{}", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&pipe_path);

    let mkfifo_out = std::process::Command::new("mkfifo")
        .arg(&pipe_path)
        .output()
        .context("Failed to run mkfifo command")?;

    if !mkfifo_out.status.success() {
        return Err(anyhow::anyhow!(
            "mkfifo failed: {}",
            String::from_utf8_lossy(&mkfifo_out.stderr)
        ));
    }

    let mut child = std::process::Command::new("pw-play")
        .arg("--properties=node.passthrough=true")
        .arg("-f")
        .arg("s16")
        .arg("-r")
        .arg(pw_rate.to_string())
        .arg("-c")
        .arg("2")
        .arg("--raw")
        .arg(&pipe_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            let _ = std::fs::remove_file(&pipe_path);
            anyhow::anyhow!("Failed to spawn pw-play. Is pipewire installed? {}", e)
        })?;

    // Check if pw-play fails immediately
    std::thread::sleep(std::time::Duration::from_millis(150));
    if let Ok(Some(status)) = child.try_wait() {
        let _ = std::fs::remove_file(&pipe_path);
        return Err(anyhow::anyhow!(
            "pw-play exited immediately with status: {}",
            status
        ));
    }

    let (video_stream_index, video_time_base, video_packet_tx, video_packet_rx) =
        if let Some(vs) = video_setup {
            let tb = vs.time_base;
            let (tx_v, rx_v) = crate::audio::spawn_video_decoder_thread(
                vs.params,
                vs.time_base,
                vs.rotation,
                shared_state.clone(),
                stop_token.clone(),
            );
            (Some(vs.stream_index), Some(tb), Some(tx_v), Some(rx_v))
        } else {
            (None, None, None, None)
        };

    let pipe_path_clone = pipe_path.clone();

    let ffmpeg_thread = std::thread::spawn(move || {
        println!("[bitstream] FFmpeg thread started on Linux.");

        struct SpdifItem {
            epoch: u64,
            packet: ffmpeg_next::Packet,
            time_base: ffmpeg_next::Rational,
            dsp_messages: Vec<crate::audio::DspMessage>,
            is_eof: bool,
        }

        let (spdif_tx, spdif_rx) = crossbeam_channel::bounded::<SpdifItem>(512);
        let pipe_path_for_pump = pipe_path_clone.clone();
        let params_for_pump = parameters.clone();
        let stop_for_pump = stop_token.clone();
        let state_for_pump = shared_state.clone();
        let tx_for_pump = tx.clone();
        let spdif_rx_for_pump = spdif_rx.clone();

        let spdif_pump_handle = std::thread::spawn(move || {
            let mut octx = match ffmpeg_next::format::output_as(&pipe_path_for_pump, "spdif") {
                Ok(ctx) => ctx,
                Err(e) => {
                    eprintln!("[bitstream] Failed to open spdif output pipe: {}", e);
                    return;
                }
            };
            let ost_index = {
                let mut ost = match octx.add_stream(ffmpeg_next::codec::Id::None) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[bitstream] Failed to add spdif stream: {}", e);
                        return;
                    }
                };
                ost.set_parameters(params_for_pump);
                ost.index()
            };

            let mut dict = ffmpeg_next::Dictionary::new();
            dict.set("flush_packets", "1");
            if octx.write_header_with(dict).is_err() {
                println!("[bitstream] Failed to write header to pipe");
                return;
            }
            let Some(ost_stream) = octx.stream(ost_index) else {
                return;
            };
            let ost_time_base = ost_stream.time_base();

            let mut sent_paused_preview_epoch: Option<u64> = None;

            loop {
                if stop_for_pump.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let (is_paused, latest_epoch) = if let Ok(state) = state_for_pump.try_lock() {
                    (state.is_paused, state.seek_epoch)
                } else {
                    (false, 0)
                };

                if is_paused {
                    if let Ok(mut state) = state_for_pump.try_lock() {
                        state.raw_channel_vus.fill(0.0);
                        state.raw_spectrum_data.fill(0.0);
                    }
                    // If a seek occurred while paused, drain stale packets and emit the first post-seek DspMessage
                    while let Ok(item) = spdif_rx_for_pump.try_recv() {
                        if item.epoch < latest_epoch {
                            continue;
                        }
                        if sent_paused_preview_epoch != Some(item.epoch)
                            && let Some(first_msg) = item.dsp_messages.first()
                        {
                            let _ = tx_for_pump.try_send(first_msg.clone());
                            sent_paused_preview_epoch = Some(item.epoch);
                        }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    continue;
                }

                match spdif_rx_for_pump.recv_timeout(std::time::Duration::from_millis(25)) {
                    Ok(mut item) => {
                        let cur_epoch = state_for_pump
                            .try_lock()
                            .map(|s| s.seek_epoch)
                            .unwrap_or(latest_epoch);
                        if item.epoch < cur_epoch {
                            continue;
                        }
                        if item.is_eof {
                            if let Ok(mut state) = state_for_pump.lock()
                                && state.seek_epoch == item.epoch
                            {
                                state.track_ended = true;
                            }
                            continue;
                        }
                        item.packet.rescale_ts(item.time_base, ost_time_base);
                        item.packet.set_stream(ost_index);
                        if item.packet.write(&mut octx).is_err() {
                            break;
                        }
                        for msg in item.dsp_messages {
                            let _ = tx_for_pump.try_send(msg);
                        }
                        if let Ok(mut state) = state_for_pump.try_lock() {
                            state.stats.audio_buffer_fill_pct =
                                ((spdif_rx_for_pump.len() as f32 / 512.0) * 100.0)
                                    .clamp(0.0, 100.0);
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = octx.write_trailer();
        });

        let decoder_context =
            match ffmpeg_next::codec::context::Context::from_parameters(parameters.clone()) {
                Ok(c) => c,
                Err(e) => {
                    println!("[bitstream] Failed to create decoder context: {}", e);
                    drop(spdif_tx);
                    let _ = child.kill();
                    let _ = spdif_pump_handle.join();
                    let _ = std::fs::remove_file(&pipe_path_clone);
                    return;
                }
            };
        let mut decoder = match decoder_context.decoder().audio() {
            Ok(d) => d,
            Err(e) => {
                println!("[bitstream] Failed to create decoder: {}", e);
                drop(spdif_tx);
                let _ = child.kill();
                let _ = spdif_pump_handle.join();
                let _ = std::fs::remove_file(&pipe_path_clone);
                return;
            }
        };
        let src_channel_layout = if decoder.channel_layout().channels() > 0 {
            decoder.channel_layout()
        } else {
            ffmpeg_next::channel_layout::ChannelLayout::default(decoder.channels().max(1) as i32)
        };
        let vis_channels = (decoder.channels() as i32).clamp(2, 8);
        let target_channel_layout =
            ffmpeg_next::channel_layout::ChannelLayout::default(vis_channels);
        let mut resampler = match ffmpeg_next::software::resampling::context::Context::get(
            decoder.format(),
            src_channel_layout,
            decoder.rate(),
            ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
            target_channel_layout,
            decoder.rate(),
        ) {
            Ok(r) => r,
            Err(e) => {
                println!("[bitstream] Failed to create resampler: {}", e);
                drop(spdif_tx);
                let _ = child.kill();
                let _ = spdif_pump_handle.join();
                let _ = std::fs::remove_file(&pipe_path_clone);
                return;
            }
        };

        let decoder_rate_u32 = decoder.rate().max(1);
        let decoder_rate = decoder_rate_u32 as f32;
        let window_size = crate::audio::calculate_power_of_two_window_size(decoder_rate_u32);
        let update_interval = (decoder_rate / 60.0).ceil() as usize;
        let mut accumulator: Vec<Vec<f32>> = Vec::new();
        let mut samples_since_last_send = 0;

        let mut current_seconds = 0.0;
        let mut current_seek_epoch = shared_state.lock().map(|s| s.seek_epoch).unwrap_or(0);
        let mut seek_in_progress = true;
        let mut target_seek_time: Option<f64> = None;
        let mut is_eof = false;
        let mut audio_pkt_bytes_accum: u64 = 0;
        let mut audio_pkt_window_start: Option<f64> = None;

        'stream_loop: loop {
            if stop_token.load(std::sync::atomic::Ordering::Relaxed) {
                println!("[bitstream] Stop token received, stopping bitstream.");
                break 'stream_loop;
            }

            // Pace lookahead decoding up to +7.5s ahead of current playback time while checking seek requests
            let mut seek_target_opt: Option<f64> = None;
            loop {
                if stop_token.load(std::sync::atomic::Ordering::Relaxed) {
                    break 'stream_loop;
                }
                let (seek_req, playback_sec) = if let Ok(mut state) = shared_state.lock() {
                    (state.seek_request.take(), state.current_seconds)
                } else {
                    (None, current_seconds)
                };
                if let Some(target_sec) = seek_req {
                    seek_target_opt = Some(target_sec);
                    break;
                }
                if is_eof
                    || (target_seek_time.is_none()
                        && current_seconds > playback_sec + 7.5
                        && spdif_rx.len() > 8)
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                break;
            }

            if let Some(target_sec) = seek_target_opt {
                if let Some(ref v_rx) = video_packet_rx {
                    while v_rx.try_recv().is_ok() {}
                }
                while spdif_rx.try_recv().is_ok() {}

                let (seek_stream_idx, target_pts) =
                    if let (Some(v_idx), Some(tb)) = (video_stream_index, video_time_base) {
                        let tb_f64 = tb.numerator() as f64 / tb.denominator().max(1) as f64;
                        (v_idx as i32, (target_sec / tb_f64).max(0.0) as i64)
                    } else {
                        (
                            -1,
                            (target_sec * ffmpeg_next::ffi::AV_TIME_BASE as f64).max(0.0) as i64,
                        )
                    };
                unsafe {
                    let ret = ffmpeg_next::ffi::av_seek_frame(
                        ictx.as_mut_ptr(),
                        seek_stream_idx,
                        target_pts,
                        ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                    );
                    if ret < 0 {
                        let fallback_pts =
                            (target_sec * ffmpeg_next::ffi::AV_TIME_BASE as f64).max(0.0) as i64;
                        ffmpeg_next::ffi::av_seek_frame(
                            ictx.as_mut_ptr(),
                            -1,
                            fallback_pts,
                            ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                        );
                    }
                }
                decoder.flush();
                for acc in &mut accumulator {
                    acc.clear();
                }
                samples_since_last_send = 0;
                current_seconds = target_sec;
                seek_in_progress = true;
                target_seek_time = Some(target_sec);
                is_eof = false;
                audio_pkt_bytes_accum = 0;
                audio_pkt_window_start = None;
                if let Ok(mut state) = shared_state.lock() {
                    state.seek_epoch += 1;
                    current_seek_epoch = state.seek_epoch;
                    state.current_seconds = target_sec;
                    state.track_ended = false;
                    state.lookahead_queue.clear();
                    state.lookahead_sample_buffer.clear();
                    state.lookahead_timeline.fill(0.0);
                    state.lookahead_buffer_start_time = target_sec;
                    state.waveform_history.clear();
                    state.raw_waveform.fill(0.0);
                }
                continue 'stream_loop;
            }

            let Some((stream, packet)) = ictx.packets().next() else {
                is_eof = true;
                let _ = spdif_tx.try_send(SpdifItem {
                    epoch: current_seek_epoch,
                    packet: ffmpeg_next::Packet::empty(),
                    time_base: ffmpeg_next::Rational::new(1, 1),
                    dsp_messages: Vec::new(),
                    is_eof: true,
                });
                continue 'stream_loop;
            };

            if Some(stream.index()) == video_stream_index {
                if let Some(ref v_tx) = video_packet_tx {
                    let _ = v_tx.try_send((current_seek_epoch, packet.clone()));
                }
                continue 'stream_loop;
            }

            if stream.index() == best_audio_index {
                let vis_packet = packet.clone();
                let pkt_tb = stream.time_base();
                audio_pkt_bytes_accum = audio_pkt_bytes_accum.saturating_add(packet.size() as u64);

                if let Some(pts) = packet.pts().or_else(|| packet.dts()) {
                    let pkt_sec = pts as f64 * f64::from(pkt_tb);
                    if seek_in_progress || (pkt_sec - current_seconds).abs() > 0.25 {
                        current_seconds = pkt_sec;
                        seek_in_progress = false;
                    }
                    if let Some(start_t) = audio_pkt_window_start {
                        let win_dur = pkt_sec - start_t;
                        if win_dur >= 0.5 {
                            let kbps = ((audio_pkt_bytes_accum as f64 * 8.0) / (win_dur * 1000.0))
                                .round() as u32;
                            if kbps > 0
                                && let Ok(mut state) = shared_state.try_lock()
                                && state.stats.audio_bitrate_kbps.is_none()
                            {
                                state.bitrate = Some(kbps);
                                state.stats.audio_bitrate_kbps = Some(kbps);
                            }
                            audio_pkt_window_start = Some(pkt_sec);
                            audio_pkt_bytes_accum = 0;
                        }
                    } else {
                        audio_pkt_window_start = Some(pkt_sec);
                        audio_pkt_bytes_accum = 0;
                    }
                }

                let mut pkt_dsp_messages = Vec::new();
                let mut skip_spdif_packet = false;

                if decoder.send_packet(&vis_packet).is_ok() {
                    let mut frame = ffmpeg_next::frame::Audio::empty();
                    while decoder.receive_frame(&mut frame).is_ok() {
                        let mut resampled = ffmpeg_next::frame::Audio::empty();
                        if resampler.run(&frame, &mut resampled).is_ok() {
                            let fresh_samples = resampled.samples();
                            let planes = resampled.channels() as usize;
                            if fresh_samples == 0 || planes == 0 {
                                continue;
                            }
                            let frame_dur = fresh_samples as f64 / decoder_rate_u32 as f64;
                            if let Some(target_pos) = target_seek_time {
                                if current_seconds + frame_dur <= target_pos {
                                    current_seconds += frame_dur;
                                    skip_spdif_packet = true;
                                    continue;
                                } else {
                                    if current_seconds < target_pos {
                                        current_seconds = target_pos;
                                    }
                                    target_seek_time = None;
                                    skip_spdif_packet = false;
                                }
                            }

                            if accumulator.len() != planes {
                                accumulator = vec![Vec::new(); planes];
                            }

                            let mut fresh_planes = Vec::with_capacity(planes);
                            for (p, acc) in accumulator.iter_mut().enumerate().take(planes) {
                                let data = &resampled.plane::<f32>(p)[..fresh_samples];
                                fresh_planes.push(data.to_vec());
                                acc.extend_from_slice(data);
                                let excess = acc.len().saturating_sub(window_size);
                                if excess > 0 {
                                    acc.drain(0..excess);
                                }
                            }

                            if let Ok(mut state) = shared_state.lock()
                                && state.seek_epoch == current_seek_epoch
                            {
                                crate::audio::push_planar_lookahead_slices(
                                    &mut state,
                                    &fresh_planes,
                                    decoder_rate_u32,
                                    current_seconds,
                                );
                            }

                            current_seconds += frame_dur;
                            samples_since_last_send += fresh_samples;
                            let acc_len = accumulator.first().map(|a| a.len()).unwrap_or(0);

                            if acc_len > 0 && samples_since_last_send >= update_interval {
                                samples_since_last_send = 0;

                                let mut channel_audio_data = Vec::with_capacity(planes);
                                let mut channel_vus = Vec::with_capacity(planes);
                                let eval_len = fresh_samples.max(update_interval).min(acc_len);

                                for acc in accumulator.iter().take(planes) {
                                    let mut peak = 0.0f32;
                                    let start_idx = acc.len().saturating_sub(eval_len);
                                    for &s in &acc[start_idx..] {
                                        peak = peak.max(s.abs());
                                    }
                                    channel_vus.push(peak.clamp(0.0, 1.0));
                                    if acc.len() == window_size {
                                        channel_audio_data.push(acc.clone());
                                    } else {
                                        let mut padded = vec![0.0f32; window_size];
                                        let off = window_size.saturating_sub(acc.len());
                                        padded[off..].copy_from_slice(acc);
                                        channel_audio_data.push(padded);
                                    }
                                }

                                let mono_audio_data = {
                                    let mut mono = vec![0.0f32; window_size];
                                    for (ch_idx, acc) in accumulator.iter().take(planes).enumerate()
                                    {
                                        let ch_weight = match ch_idx {
                                            0 | 1 => {
                                                if planes == 1 {
                                                    1.0
                                                } else {
                                                    0.5
                                                }
                                            }
                                            2 => std::f32::consts::FRAC_1_SQRT_2,
                                            _ => 0.25,
                                        };
                                        let off = window_size.saturating_sub(acc.len());
                                        for (i, &sample) in acc.iter().take(window_size).enumerate()
                                        {
                                            mono[off + i] += sample * ch_weight;
                                        }
                                    }
                                    for s in &mut mono {
                                        *s = s.clamp(-1.0, 1.0);
                                    }
                                    mono
                                };

                                pkt_dsp_messages.push(crate::audio::DspMessage {
                                    audio_data: mono_audio_data,
                                    channel_vus,
                                    current_order: 0,
                                    current_row: 0,
                                    bpm: 0,
                                    speed: 0,
                                    current_seconds,
                                    current_row_string: String::new(),
                                    channel_audio_data,
                                    seek_epoch: current_seek_epoch,
                                });
                            }
                        }
                    }
                }

                if !skip_spdif_packet {
                    let mut pending_item = Some(SpdifItem {
                        epoch: current_seek_epoch,
                        packet,
                        time_base: pkt_tb,
                        dsp_messages: pkt_dsp_messages,
                        is_eof: false,
                    });
                    while let Some(item) = pending_item.take() {
                        if stop_token.load(std::sync::atomic::Ordering::Relaxed) {
                            break 'stream_loop;
                        }
                        if let Ok(state) = shared_state.try_lock()
                            && (state.seek_request.is_some() || state.seek_epoch > item.epoch)
                        {
                            break;
                        }
                        match spdif_tx.send_timeout(item, std::time::Duration::from_millis(15)) {
                            Ok(()) => break,
                            Err(crossbeam_channel::SendTimeoutError::Timeout(it)) => {
                                pending_item = Some(it);
                            }
                            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                                break 'stream_loop;
                            }
                        }
                    }
                }
            }
        }
        drop(spdif_tx);
        let _ = child.kill();
        let _ = child.wait();
        let _ = spdif_pump_handle.join();
        let _ = std::fs::remove_file(&pipe_path_clone);
        println!("[bitstream] FFmpeg thread finished on Linux.");
    });

    Ok((
        ffmpeg_thread,
        decoder_sample_rate as u32,
        8u16,
        codec_name,
        has_video,
        meta_artist,
        meta_duration,
    ))
}

#[cfg(target_os = "macos")]
mod macos_bitstream {
    use crate::audio::DspMessage;
    use crate::state::AppState;
    use anyhow::Result;
    use crossbeam_channel::Sender;
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    pub fn start_bitstream_thread(
        _file_path: &str,
        _shared_state: Arc<Mutex<AppState>>,
        _tx: Sender<DspMessage>,
        _stop_token: Arc<AtomicBool>,
    ) -> Result<super::BitstreamLaunchResult> {
        Err(anyhow::anyhow!(
            "macOS CoreAudio bitstream HAL packetization (IEC 61937) is not supported. Standard decoded PCM playback will be used."
        ))
    }
}

#[cfg(target_os = "macos")]
pub use macos_bitstream::start_bitstream_thread;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn start_bitstream_thread(
    _file_path: &str,
    _shared_state: std::sync::Arc<std::sync::Mutex<crate::state::AppState>>,
    _tx: crossbeam_channel::Sender<crate::audio::DspMessage>,
    _stop_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<BitstreamLaunchResult> {
    Err(anyhow::anyhow!(
        "Bitstream passthrough is not supported on this platform."
    ))
}
#[cfg(windows)]
pub use wasapi_bitstream::start_bitstream_thread;

#[cfg(windows)]
mod wasapi_bitstream {
    use crate::audio::DspMessage;
    use crate::state::AppState;
    use anyhow::{Context, Result};
    use crossbeam_channel::Sender;
    use std::io::Read;
    use std::ptr;
    use std::sync::{Arc, Mutex, atomic::AtomicBool};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::core::GUID;

    const WAIT_OBJECT_0: u32 = 0;
    static PIPE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    // ─── Standard PCM & IEEE Float WASAPI SubFormat GUIDs ──────────
    const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID {
        data1: 0x00000001,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID {
        data1: 0x00000003,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    // ─── IEC 61937 WASAPI SubFormat GUIDs ────────────────────────────
    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL: GUID = GUID {
        data1: 0x00000092,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS: GUID = GUID {
        data1: 0x0000000a,
        data2: 0x0cea,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20: GUID = GUID {
        data1: 0x00000017,
        data2: 0x0cea,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DTS: GUID = GUID {
        data1: 0x00000008,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD: GUID = GUID {
        data1: 0x0000000b,
        data2: 0x0cea,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MLP: GUID = GUID {
        data1: 0x0000000c,
        data2: 0x0cea,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const PKEY_DEVICE_FRIENDLY_NAME: windows::Win32::Foundation::PROPERTYKEY =
        windows::Win32::Foundation::PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum WasapiStreamType {
        CompressedBitstream,
        Lpcm,
    }

    #[derive(Debug, Clone)]
    struct AudioProfile {
        name: String,
        stream_type: WasapiStreamType,
        channels: u16,
        rate: u32,
        valid_bits: u16,
        container_bits: u16,
        channel_mask: u32,
        sub_format: GUID,
        sample_format: ffmpeg_next::format::sample::Sample,
    }

    fn get_device_name(device: &IMMDevice) -> Option<String> {
        unsafe {
            let store = device
                .OpenPropertyStore(windows::Win32::System::Com::STGM_READ)
                .ok()?;
            let propvar = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
            if propvar.Anonymous.Anonymous.vt == windows::Win32::System::Variant::VT_LPWSTR {
                let pwstr = propvar.Anonymous.Anonymous.Anonymous.pwszVal;
                if !pwstr.is_null() {
                    pwstr.to_string().ok()
                } else {
                    None
                }
            } else {
                let s = propvar.to_string();
                if s.is_empty() { None } else { Some(s) }
            }
        }
    }

    fn initialize_exclusive_client(
        device: &IMMDevice,
        profile: &AudioProfile,
        format: &[u8],
    ) -> anyhow::Result<(IAudioClient, IAudioRenderClient, HANDLE, u32)> {
        let mut audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let mut default_period = 0;
        let mut min_period = 0;
        unsafe {
            audio_client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))?;
        }
        let mut buffer_duration = if min_period > 0 {
            min_period
        } else {
            default_period.max(100_000)
        };
        let mut hr = unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                buffer_duration,
                buffer_duration,
                format.as_ptr() as *const _,
                None,
            )
        };
        if let Err(e) = &hr {
            if e.code() == windows::core::HRESULT(0x88890019u32 as i32)
                || e.code() == windows::core::HRESULT(0x80070057u32 as i32)
            {
                let aligned_frames = unsafe { audio_client.GetBufferSize().unwrap_or(0) };
                if aligned_frames > 0 {
                    buffer_duration = (aligned_frames as i64 * 10_000_000) / profile.rate as i64;
                    let audio_client_new: IAudioClient =
                        unsafe { device.Activate(CLSCTX_ALL, None)? };
                    hr = unsafe {
                        audio_client_new.Initialize(
                            AUDCLNT_SHAREMODE_EXCLUSIVE,
                            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                            buffer_duration,
                            buffer_duration,
                            format.as_ptr() as *const _,
                            None,
                        )
                    };
                    if hr.is_ok() {
                        audio_client = audio_client_new;
                    } else {
                        hr.map_err(|e| anyhow::anyhow!(e))?;
                    }
                } else {
                    hr.map_err(|e| anyhow::anyhow!(e))?;
                }
            } else {
                hr.map_err(|e| anyhow::anyhow!(e))?;
            }
        }
        let event = unsafe { CreateEventW(None, false, false, None)? };
        if let Err(e) = unsafe { audio_client.SetEventHandle(event) } {
            unsafe {
                let _ = CloseHandle(event);
            }
            return Err(anyhow::anyhow!(e));
        }
        let buffer_frames = unsafe { audio_client.GetBufferSize()? };
        let render_client: IAudioRenderClient = match unsafe { audio_client.GetService() } {
            Ok(rc) => rc,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(event);
                }
                return Err(anyhow::anyhow!(e));
            }
        };
        Ok((audio_client, render_client, event, buffer_frames))
    }

    fn detect_codec_profile(
        codec_id: ffmpeg_next::codec::Id,
        src_channels: u16,
        src_rate: u32,
    ) -> Result<(String, WasapiStreamType, Vec<AudioProfile>)> {
        match codec_id {
            ffmpeg_next::codec::Id::TRUEHD => Ok((
                "truehd".to_string(),
                WasapiStreamType::CompressedBitstream,
                vec![
                    AudioProfile {
                        name: "TrueHD / Dolby Atmos (MAT 2.0 HBR)".to_string(),
                        stream_type: WasapiStreamType::CompressedBitstream,
                        channels: 8,
                        rate: 192000,
                        valid_bits: 16,
                        container_bits: 16,
                        channel_mask: 0x63F,
                        sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20,
                        sample_format: ffmpeg_next::format::sample::Sample::None,
                    },
                    AudioProfile {
                        name: "TrueHD / Dolby Atmos (MLP HBR)".to_string(),
                        stream_type: WasapiStreamType::CompressedBitstream,
                        channels: 8,
                        rate: 192000,
                        valid_bits: 16,
                        container_bits: 16,
                        channel_mask: 0x63F,
                        sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MLP,
                        sample_format: ffmpeg_next::format::sample::Sample::None,
                    },
                ],
            )),
            ffmpeg_next::codec::Id::EAC3 => Ok((
                "eac3".to_string(),
                WasapiStreamType::CompressedBitstream,
                vec![AudioProfile {
                    name: "E-AC3 / Dolby Digital Plus".to_string(),
                    stream_type: WasapiStreamType::CompressedBitstream,
                    channels: 2,
                    rate: 192000,
                    valid_bits: 16,
                    container_bits: 16,
                    channel_mask: 0x3,
                    sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS,
                    sample_format: ffmpeg_next::format::sample::Sample::None,
                }],
            )),
            ffmpeg_next::codec::Id::DTS => Ok((
                "dts".to_string(),
                WasapiStreamType::CompressedBitstream,
                vec![
                    AudioProfile {
                        name: "DTS-HD MA (HBR)".to_string(),
                        stream_type: WasapiStreamType::CompressedBitstream,
                        channels: 8,
                        rate: 192000,
                        valid_bits: 16,
                        container_bits: 16,
                        channel_mask: 0x63F,
                        sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD,
                        sample_format: ffmpeg_next::format::sample::Sample::None,
                    },
                    AudioProfile {
                        name: "DTS Core (fallback)".to_string(),
                        stream_type: WasapiStreamType::CompressedBitstream,
                        channels: 2,
                        rate: 48000,
                        valid_bits: 16,
                        container_bits: 16,
                        channel_mask: 0x3,
                        sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DTS,
                        sample_format: ffmpeg_next::format::sample::Sample::None,
                    },
                ],
            )),
            ffmpeg_next::codec::Id::AC3 => Ok((
                "ac3".to_string(),
                WasapiStreamType::CompressedBitstream,
                vec![AudioProfile {
                    name: "AC3 / Dolby Digital".to_string(),
                    stream_type: WasapiStreamType::CompressedBitstream,
                    channels: 2,
                    rate: 48000,
                    valid_bits: 16,
                    container_bits: 16,
                    channel_mask: 0x3,
                    sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL,
                    sample_format: ffmpeg_next::format::sample::Sample::None,
                }],
            )),
            // Uncompressed Multi-Channel LPCM (only when >2 channels; stereo uses shared mode)
            // Uncompressed Multi-Channel LPCM now uses WASAPI Shared Mode via cpal; Exclusive Mode is restricted to compressed bitstreams
            id if false && src_channels > 2 => {
                let codec_name = match id {
                    ffmpeg_next::codec::Id::FLAC => "flac",
                    ffmpeg_next::codec::Id::ALAC => "alac",
                    ffmpeg_next::codec::Id::WAVPACK => "wavpack",
                    _ if format!("{:?}", id).starts_with("PCM_") => "pcm",
                    _ => "multichannel_pcm",
                }
                .to_string();

                let mut profiles = Vec::new();
                let rates_to_try = if src_rate != 48000 {
                    vec![src_rate, 48000]
                } else {
                    vec![src_rate]
                };

                let channel_configs: Vec<(u16, u32)> = match src_channels {
                    6 => vec![(6, 0x3F), (6, 0x60F), (8, 0x63F)],
                    8 => vec![(8, 0x63F), (6, 0x3F)],
                    4 => vec![(4, 0x33), (6, 0x3F)],
                    2 => vec![(2, 0x3)],
                    _ => vec![(src_channels, if src_channels <= 6 { 0x3F } else { 0x63F })],
                };

                for &rate in &rates_to_try {
                    for &(ch, mask) in &channel_configs {
                        profiles.push(AudioProfile {
                            name: format!(
                                "Multi-Channel LPCM 32-bit Float ({}ch x {}Hz, mask 0x{:X})",
                                ch, rate, mask
                            ),
                            stream_type: WasapiStreamType::Lpcm,
                            channels: ch,
                            rate,
                            valid_bits: 32,
                            container_bits: 32,
                            channel_mask: mask,
                            sub_format: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
                            sample_format: ffmpeg_next::format::sample::Sample::F32(
                                ffmpeg_next::format::sample::Type::Packed,
                            ),
                        });
                        profiles.push(AudioProfile {
                            name: format!(
                                "Multi-Channel LPCM 16-bit ({}ch x {}Hz, mask 0x{:X})",
                                ch, rate, mask
                            ),
                            stream_type: WasapiStreamType::Lpcm,
                            channels: ch,
                            rate,
                            valid_bits: 16,
                            container_bits: 16,
                            channel_mask: mask,
                            sub_format: KSDATAFORMAT_SUBTYPE_PCM,
                            sample_format: ffmpeg_next::format::sample::Sample::I16(
                                ffmpeg_next::format::sample::Type::Packed,
                            ),
                        });
                        profiles.push(AudioProfile {
                            name: format!(
                                "Multi-Channel LPCM 24-bit ({}ch x {}Hz, mask 0x{:X})",
                                ch, rate, mask
                            ),
                            stream_type: WasapiStreamType::Lpcm,
                            channels: ch,
                            rate,
                            valid_bits: 24,
                            container_bits: 32,
                            channel_mask: mask,
                            sub_format: KSDATAFORMAT_SUBTYPE_PCM,
                            sample_format: ffmpeg_next::format::sample::Sample::I32(
                                ffmpeg_next::format::sample::Type::Packed,
                            ),
                        });
                    }
                }
                Ok((codec_name, WasapiStreamType::Lpcm, profiles))
            }
            _ => Err(anyhow::anyhow!(
                "Unsupported codec for bitstreaming or exclusive multi-channel LPCM: {:?}",
                codec_id
            )),
        }
    }

    #[repr(C, packed)]
    struct WAVEFORMATEXTENSIBLE_IEC61937 {
        format_ext: WAVEFORMATEXTENSIBLE,
        dw_encoded_samples_per_sec: u32,
        dw_encoded_channel_count: u32,
        dw_average_bytes_per_sec: u32,
    }

    fn build_format(profile: &AudioProfile) -> Vec<u8> {
        let block_align = profile.channels * (profile.container_bits / 8);
        let avg_bytes = profile.rate * block_align as u32;

        match profile.stream_type {
            WasapiStreamType::CompressedBitstream => {
                let is_hbr = profile.rate > 48000;
                let format_ext = WAVEFORMATEXTENSIBLE {
                    Format: WAVEFORMATEX {
                        wFormatTag: 0xFFFE, // WAVE_FORMAT_EXTENSIBLE
                        nChannels: profile.channels,
                        nSamplesPerSec: profile.rate,
                        nAvgBytesPerSec: avg_bytes,
                        nBlockAlign: block_align,
                        wBitsPerSample: 16,
                        cbSize: if is_hbr { 34 } else { 22 },
                    },
                    Samples: WAVEFORMATEXTENSIBLE_0 {
                        wValidBitsPerSample: 16,
                    },
                    dwChannelMask: profile.channel_mask,
                    SubFormat: profile.sub_format,
                };

                if is_hbr {
                    let iec = WAVEFORMATEXTENSIBLE_IEC61937 {
                        format_ext,
                        dw_encoded_samples_per_sec: profile.rate,
                        dw_encoded_channel_count: profile.channels as u32,
                        dw_average_bytes_per_sec: avg_bytes,
                    };
                    unsafe {
                        std::slice::from_raw_parts(
                            &iec as *const _ as *const u8,
                            std::mem::size_of::<WAVEFORMATEXTENSIBLE_IEC61937>(),
                        )
                        .to_vec()
                    }
                } else {
                    unsafe {
                        std::slice::from_raw_parts(
                            &format_ext as *const _ as *const u8,
                            std::mem::size_of::<WAVEFORMATEXTENSIBLE>(),
                        )
                        .to_vec()
                    }
                }
            }
            WasapiStreamType::Lpcm => {
                let format_ext = WAVEFORMATEXTENSIBLE {
                    Format: WAVEFORMATEX {
                        wFormatTag: 0xFFFE, // WAVE_FORMAT_EXTENSIBLE
                        nChannels: profile.channels,
                        nSamplesPerSec: profile.rate,
                        nAvgBytesPerSec: avg_bytes,
                        nBlockAlign: block_align,
                        wBitsPerSample: profile.container_bits,
                        cbSize: 22,
                    },
                    Samples: WAVEFORMATEXTENSIBLE_0 {
                        wValidBitsPerSample: profile.valid_bits,
                    },
                    dwChannelMask: profile.channel_mask,
                    SubFormat: profile.sub_format,
                };

                unsafe {
                    std::slice::from_raw_parts(
                        &format_ext as *const _ as *const u8,
                        std::mem::size_of::<WAVEFORMATEXTENSIBLE>(),
                    )
                    .to_vec()
                }
            }
        }
    }

    pub fn list_devices() -> Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
        let count = unsafe { collection.GetCount()? };

        println!("Available audio render endpoints:\n");
        for i in 0..count {
            if let Ok(device) = unsafe { collection.Item(i) } {
                let id = unsafe { device.GetId()?.to_string()? };
                let name = get_device_name(&device).unwrap_or_else(|| "Unknown".to_string());
                println!("  [{}] {} (ID: {})", i, name, id);
            }
        }
        println!("\nUse --device <N> or settings to select a specific endpoint.");

        drop(collection);
        drop(enumerator);
        unsafe {
            CoUninitialize();
        }
        Ok(())
    }

    pub fn start_bitstream_thread(
        file_path: &str,
        shared_state: Arc<Mutex<AppState>>,
        tx: Sender<DspMessage>,
        stop_token: Arc<AtomicBool>,
    ) -> Result<super::BitstreamLaunchResult> {
        if shared_state
            .lock()
            .map(|s| s.force_stereo_downmix)
            .unwrap_or(false)
        {
            return Err(anyhow::anyhow!(
                "Force stereo downmix enabled; skipping WASAPI Exclusive"
            ));
        }

        // ── Probe codec via ffmpeg-next (pure FFmpeg, NO COM on caller thread) ──
        println!("[bitstream] Probing audio stream via ffmpeg-next on Windows...");
        ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
        ffmpeg_next::init().context("Failed to initialize ffmpeg-next")?;

        let mut dict = ffmpeg_next::Dictionary::new();
        dict.set("probesize", "5000000");
        dict.set("analyzeduration", "5000000");
        let mut ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
            .context("Failed to open input file")?;

        let (codec_id, best_audio_index, parameters, stream_time_base, mut audio_bitrate_kbps) = {
            let best_audio = ictx
                .streams()
                .best(ffmpeg_next::media::Type::Audio)
                .ok_or_else(|| anyhow::anyhow!("No audio stream found"))?;
            (
                best_audio.parameters().id(),
                best_audio.index(),
                best_audio.parameters(),
                f64::from(best_audio.time_base()),
                crate::audio::probe_stream_bitrate_kbps(&best_audio),
            )
        };

        let probe_ctx = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone())
            .context("Failed to create probe context")?;
        let probe_decoder = probe_ctx
            .decoder()
            .audio()
            .context("Failed to create probe decoder")?;
        let decoder_sample_rate = probe_decoder.rate();
        let src_channels = probe_decoder.channels();
        if audio_bitrate_kbps.is_none() && probe_decoder.bit_rate() > 0 {
            audio_bitrate_kbps = Some((probe_decoder.bit_rate() / 1000).max(1) as u32);
        }

        let (codec_name, stream_type, profiles) =
            detect_codec_profile(codec_id, src_channels, decoder_sample_rate)?;
        let video_setup = crate::audio::probe_video_stream(&ictx);
        let has_video = video_setup.is_some();
        let video_fps = video_setup.as_ref().map(|vs| vs.fps).unwrap_or(0.0);
        let mut video_bitrate_kbps = video_setup.as_ref().and_then(|vs| vs.bitrate_kbps);
        let container_kbps = if ictx.bit_rate() > 0 {
            Some((ictx.bit_rate() / 1000).max(1) as u32)
        } else {
            None
        };
        if video_bitrate_kbps.is_none()
            && has_video
            && let Some(c_kbps) = container_kbps
        {
            let a_kbps = audio_bitrate_kbps.unwrap_or(0);
            video_bitrate_kbps = Some(if c_kbps > a_kbps {
                c_kbps - a_kbps
            } else {
                c_kbps
            });
        }
        if audio_bitrate_kbps.is_none()
            && let Some(c_kbps) = container_kbps
        {
            if !has_video {
                audio_bitrate_kbps = Some(c_kbps);
            } else if let Some(v_kbps) = video_bitrate_kbps
                && c_kbps > v_kbps
            {
                audio_bitrate_kbps = Some(c_kbps - v_kbps);
            }
        }
        if let Ok(mut state) = shared_state.lock() {
            state.bitrate = audio_bitrate_kbps;
            state.stats.audio_bitrate_kbps = audio_bitrate_kbps;
            state.stats.video_fps = video_fps;
            state.stats.video_bitrate_kbps = video_bitrate_kbps;
        }
        let meta_duration = if ictx.duration() > 0 {
            ictx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64
        } else {
            0.0
        };
        let meta_artist = {
            let meta = ictx.metadata();
            meta.get("artist")
                .or_else(|| meta.get("ARTIST"))
                .map(|s| s.to_string())
                .unwrap_or_default()
        };

        println!(
            "[bitstream] Detected codec: {}, Decoder Rate: {} Hz, Channels: {}",
            codec_name, decoder_sample_rate, src_channels
        );
        for p in &profiles {
            println!(
                "  [bitstream] Candidate profile: {} ({}ch x {}Hz)",
                p.name, p.channels, p.rate
            );
        }

        let (init_tx, init_rx) =
            crossbeam_channel::bounded::<Result<(u32, u16, String, bool, String, f64)>>(1);
        let stop_token_pump = stop_token.clone();
        let shared_state_pump = shared_state.clone();
        let codec_name_pump = codec_name.clone();
        let meta_artist_pump = meta_artist.clone();
        let meta_duration_pump = meta_duration;

        let handle = std::thread::spawn(move || {
            // Initialize COM strictly on this dedicated thread
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }

            let setup_result: Result<_> = (|| {
                let enumerator: IMMDeviceEnumerator =
                    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };

                let selected_device_name = shared_state_pump
                    .lock()
                    .ok()
                    .and_then(|s| s.selected_audio_device.clone());
                println!(
                    "[bitstream] Selected audio device in state: {:?}",
                    selected_device_name
                );

                let collection =
                    unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
                let count = unsafe { collection.GetCount()? };

                let mut target_device: Option<IMMDevice> = None;
                if let Some(target) = &selected_device_name {
                    let target_lower = target.to_lowercase();
                    for i in 0..count {
                        if let Ok(dev) = unsafe { collection.Item(i) } {
                            let dev_name = get_device_name(&dev).unwrap_or_default();
                            let dev_id = unsafe {
                                dev.GetId()
                                    .ok()
                                    .and_then(|id| id.to_string().ok())
                                    .unwrap_or_default()
                            };
                            if dev_name.to_lowercase().contains(&target_lower)
                                || target_lower.contains(&dev_name.to_lowercase())
                                || dev_id == *target
                            {
                                println!(
                                    "[bitstream] Found matching device for '{}': {}",
                                    target, dev_name
                                );
                                target_device = Some(dev);
                                break;
                            }
                        }
                    }
                }

                let default_device = unsafe {
                    enumerator
                        .GetDefaultAudioEndpoint(eRender, eMultimedia)
                        .ok()
                };

                // Helper to test if a device accepts any of the given profiles
                let test_device_formats =
                    |dev: &IMMDevice,
                     profs: &[AudioProfile]|
                     -> Option<(IAudioClient, AudioProfile, Vec<u8>)> {
                        let client: IAudioClient = unsafe { dev.Activate(CLSCTX_ALL, None).ok()? };
                        for p in profs {
                            let fmt = build_format(p);
                            let hr = unsafe {
                                client.IsFormatSupported(
                                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                                    fmt.as_ptr() as *const _,
                                    None,
                                )
                            };
                            if hr.is_ok() {
                                println!(
                                    "  [bitstream] Device accepted profile: {} ({}ch x {}Hz)",
                                    p.name, p.channels, p.rate
                                );
                                return Some((client, p.clone(), fmt));
                            }
                        }
                        None
                    };

                let mut negotiated = if let Some(ref dev) = target_device {
                    println!(
                        "[bitstream] Testing selected audio device for exclusive/bitstream support..."
                    );
                    test_device_formats(dev, &profiles).map(|(c, p, f)| (dev.clone(), c, p, f))
                } else if let Some(ref dev) = default_device {
                    println!(
                        "[bitstream] Testing default audio endpoint for exclusive/bitstream support..."
                    );
                    test_device_formats(dev, &profiles).map(|(c, p, f)| (dev.clone(), c, p, f))
                } else {
                    None
                };

                // If target/default endpoint rejected the formats, probe other active endpoints
                if false && negotiated.is_none() {
                    println!(
                        "[bitstream] Default/selected endpoint rejected formats; probing other active endpoints for AVR/HDMI..."
                    );
                    for i in 0..count {
                        if let Ok(dev) = unsafe { collection.Item(i) } {
                            let dev_name = get_device_name(&dev).unwrap_or_default();
                            if let Some((c, p, f)) = test_device_formats(&dev, &profiles) {
                                println!(
                                    "[bitstream] Selected capable endpoint [{}]: {}",
                                    i, dev_name
                                );
                                negotiated = Some((dev, c, p, f));
                                break;
                            }
                        }
                    }
                }

                drop(collection);
                drop(enumerator);
                drop(target_device);
                drop(default_device);

                let (device, mut audio_client, profile, format) = negotiated
                    .ok_or_else(|| anyhow::anyhow!("No exclusive/bitstream format accepted by available audio endpoints.\n\
                        Ensure your audio receiver supports multichannel LPCM or bitstream output over HDMI/SPDIF."))?;

                println!(
                    "\n[bitstream] Initializing audio client for {}...",
                    profile.name
                );

                let mut default_period = 0;
                let mut min_period = 0;
                unsafe {
                    audio_client
                        .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))?;
                }

                println!(
                    "  Device Periods: Default = {}ns, Min = {}ns",
                    default_period * 100,
                    min_period * 100
                );

                let mut buffer_duration = if min_period > 0 {
                    min_period
                } else {
                    default_period.max(100_000)
                };

                let mut hr = unsafe {
                    audio_client.Initialize(
                        AUDCLNT_SHAREMODE_EXCLUSIVE,
                        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                        buffer_duration,
                        buffer_duration,
                        format.as_ptr() as *const _,
                        None,
                    )
                };

                if let Err(e) = &hr {
                    if e.code() == windows::core::HRESULT(0x88890019u32 as i32)
                        || e.code() == windows::core::HRESULT(0x80070057u32 as i32)
                    {
                        println!(
                            "  Initialize rejected duration {} (HRESULT: {:?}). Attempting to align...",
                            buffer_duration,
                            e.code()
                        );

                        let aligned_frames = unsafe { audio_client.GetBufferSize().unwrap_or(0) };
                        if aligned_frames > 0 {
                            buffer_duration =
                                (aligned_frames as i64 * 10_000_000) / profile.rate as i64;
                            println!(
                                "  Aligned buffer duration: {}ns ({} frames)",
                                buffer_duration * 100,
                                aligned_frames
                            );

                            let audio_client_new: IAudioClient =
                                unsafe { device.Activate(CLSCTX_ALL, None)? };
                            hr = unsafe {
                                audio_client_new.Initialize(
                                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                                    buffer_duration,
                                    buffer_duration,
                                    format.as_ptr() as *const _,
                                    None,
                                )
                            };

                            if hr.is_ok() {
                                audio_client = audio_client_new;
                            } else {
                                hr.map_err(|e| anyhow::anyhow!(e))?;
                            }
                        } else {
                            hr.map_err(|e| anyhow::anyhow!(e))?;
                        }
                    } else {
                        hr.map_err(|e| anyhow::anyhow!(e))?;
                    }
                }

                // Keep `device` alive so Exclusive Mode can release on pause and re-acquire on resume

                let event = unsafe { CreateEventW(None, false, false, None)? };
                if let Err(e) = unsafe { audio_client.SetEventHandle(event) } {
                    unsafe {
                        let _ = CloseHandle(event);
                    }
                    return Err(anyhow::anyhow!(e));
                }

                let buffer_frames = unsafe { audio_client.GetBufferSize()? };
                let bytes_per_sample = (profile.container_bits / 8) as u32;
                let frame_bytes = profile.channels as u32 * bytes_per_sample;
                let render_client: IAudioRenderClient = match unsafe { audio_client.GetService() } {
                    Ok(rc) => rc,
                    Err(e) => {
                        unsafe {
                            let _ = CloseHandle(event);
                        }
                        return Err(anyhow::anyhow!(e));
                    }
                };
                println!(
                    "Buffer: {} frames ({:.1} ms)",
                    buffer_frames,
                    buffer_frames as f64 / profile.rate as f64 * 1000.0
                );

                #[derive(Clone)]
                struct AudioPacket {
                    pcm_bytes: Vec<u8>,
                    epoch: u64,
                    is_eof: bool,
                }

                let (pcm_tx, pcm_rx) = crossbeam_channel::bounded::<AudioPacket>(1024);
                let (vis_tx, vis_rx) = crossbeam_channel::bounded::<DspMessage>(1024);

                let stop_token_ffmpeg = stop_token_pump.clone();
                let profile_clone = profile.clone();

                let (video_stream_index, video_time_base, video_packet_tx, video_packet_rx) =
                    if let Some(vs) = video_setup {
                        let tb = vs.time_base;
                        let (tx_v, rx_v) = crate::audio::spawn_video_decoder_thread(
                            vs.params,
                            vs.time_base,
                            vs.rotation,
                            shared_state_pump.clone(),
                            stop_token_pump.clone(),
                        );
                        (Some(vs.stream_index), Some(tb), Some(tx_v), Some(rx_v))
                    } else {
                        (None, None, None, None)
                    };

                let ffmpeg_thread = match stream_type {
                    WasapiStreamType::CompressedBitstream => {
                        use std::sync::atomic::Ordering;
                        use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
                        use windows::Win32::System::Pipes::{
                            ConnectNamedPipe, CreateNamedPipeA, NAMED_PIPE_MODE, PeekNamedPipe,
                        };

                        let pipe_id =
                            PIPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let pipe_name = format!(
                            "\\\\.\\pipe\\rusttracker_bitstream_{}_{}",
                            std::process::id(),
                            pipe_id
                        );
                        let pipe_name_nul = format!("{}\0", pipe_name);

                        let pipe_handle = match unsafe {
                            CreateNamedPipeA(
                                windows::core::PCSTR::from_raw(pipe_name_nul.as_ptr()),
                                FILE_FLAGS_AND_ATTRIBUTES(1), // PIPE_ACCESS_INBOUND
                                NAMED_PIPE_MODE(0), // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT
                                255,                // PIPE_UNLIMITED_INSTANCES
                                65536,
                                65536,
                                0,
                                None,
                            )
                        } {
                            Ok(h) => h,
                            Err(e) => {
                                drop(render_client);
                                drop(audio_client);
                                unsafe {
                                    let _ = CloseHandle(event);
                                }
                                return Err(anyhow::anyhow!(e));
                            }
                        };

                        let pipe_name_clone = pipe_name.clone();
                        let stop_token_worker = stop_token_pump.clone();
                        let vis_tx_worker = vis_tx.clone();
                        let pcm_tx_worker = pcm_tx.clone();
                        let shared_state_worker = shared_state_pump.clone();
                        let eof_flag = Arc::new(AtomicBool::new(false));
                        let eof_flag_worker = eof_flag.clone();
                        let eof_flag_feeder = eof_flag.clone();
                        let initial_epoch =
                            shared_state_pump.lock().map(|s| s.seek_epoch).unwrap_or(0);
                        let writer_epoch =
                            Arc::new(std::sync::atomic::AtomicU64::new(initial_epoch));
                        let writer_epoch_worker = writer_epoch.clone();
                        let writer_epoch_feeder = writer_epoch.clone();
                        let pipe_flushing = Arc::new(AtomicBool::new(false));
                        let pipe_flushing_worker = pipe_flushing.clone();
                        let pipe_flushing_feeder = pipe_flushing.clone();

                        let ffmpeg_worker = std::thread::spawn(move || {
                            println!(
                                "[bitstream] Compressed bitstream FFmpeg worker thread started."
                            );

                            let mut octx =
                                match ffmpeg_next::format::output_as(&pipe_name_clone, "spdif") {
                                    Ok(ctx) => ctx,
                                    Err(e) => {
                                        eprintln!("[bitstream] Failed to open spdif muxer: {}", e);
                                        let _ = std::fs::OpenOptions::new()
                                            .write(true)
                                            .open(&pipe_name_clone);
                                        return;
                                    }
                                };
                            let ost_index = {
                                let mut ost = match octx.add_stream(ffmpeg_next::codec::Id::None) {
                                    Ok(st) => st,
                                    Err(e) => {
                                        eprintln!("[bitstream] Failed to add stream: {}", e);
                                        return;
                                    }
                                };
                                ost.set_parameters(parameters.clone());
                                ost.index()
                            };

                            let mut dict = ffmpeg_next::Dictionary::new();
                            dict.set("flush_packets", "1");
                            if let Err(e) = octx.write_header_with(dict) {
                                eprintln!("[bitstream] Failed to write header: {}", e);
                                return;
                            }

                            let decoder_context =
                                match ffmpeg_next::codec::context::Context::from_parameters(
                                    parameters.clone(),
                                ) {
                                    Ok(ctx) => ctx,
                                    Err(e) => {
                                        eprintln!(
                                            "[bitstream] Failed to create visualizer decoder context: {}",
                                            e
                                        );
                                        return;
                                    }
                                };
                            let mut decoder = match decoder_context.decoder().audio() {
                                Ok(dec) => dec,
                                Err(e) => {
                                    eprintln!(
                                        "[bitstream] Failed to create visualizer audio decoder: {}",
                                        e
                                    );
                                    return;
                                }
                            };

                            let vis_channels = (decoder.channels() as i32).clamp(2, 8);
                            let target_channel_layout =
                                ffmpeg_next::channel_layout::ChannelLayout::default(vis_channels);
                            let target_ch = vis_channels as usize;

                            let decoder_rate = decoder.rate() as f32;
                            let window_size =
                                crate::audio::calculate_power_of_two_window_size(decoder.rate());
                            let update_interval = ((decoder_rate / 60.0).round() as usize).max(256);
                            let mut accumulator: Vec<Vec<f32>> =
                                vec![vec![0.0f32; window_size]; target_ch];
                            let mut samples_since_last_send = 0usize;
                            let mut current_seconds = 0.0;
                            let mut seek_in_progress = true;
                            let mut target_seek_time: Option<f64> = None;

                            let mut resampler: Option<
                                ffmpeg_next::software::resampling::context::Context,
                            > = None;
                            let mut cur_in_fmt = ffmpeg_next::format::sample::Sample::None;
                            let mut cur_in_layout =
                                ffmpeg_next::channel_layout::ChannelLayout::default(0);
                            let mut cur_in_rate = 0;

                            let mut current_seek_epoch = shared_state_worker
                                .lock()
                                .map(|s| s.seek_epoch)
                                .unwrap_or(0);
                            let mut is_eof = false;
                            let mut audio_pkt_bytes_accum: u64 = 0;
                            let mut audio_pkt_window_start: Option<f64> = None;

                            loop {
                                if stop_token_worker.load(std::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }

                                let (seek_req, playback_sec) = {
                                    if let Ok(mut state) = shared_state_worker.lock() {
                                        if let Some(target) = state.seek_request.take() {
                                            state.seek_epoch += 1;
                                            state.current_seconds = target;
                                            state.track_ended = false;
                                            state.lookahead_queue.clear();
                                            state.lookahead_sample_buffer.clear();
                                            state.lookahead_timeline.fill(0.0);
                                            state.lookahead_buffer_start_time = target;
                                            state.waveform_history.clear();
                                            state.raw_waveform.fill(0.0);
                                            (Some((target, state.seek_epoch)), target)
                                        } else {
                                            (None, state.current_seconds)
                                        }
                                    } else {
                                        (None, current_seconds)
                                    }
                                };

                                if let Some((pos, epoch)) = seek_req {
                                    pipe_flushing_worker.store(true, Ordering::SeqCst);
                                    current_seek_epoch = epoch;
                                    is_eof = false;
                                    eof_flag_worker.store(false, Ordering::SeqCst);
                                    seek_in_progress = true;
                                    target_seek_time = Some(pos);
                                    samples_since_last_send = 0;
                                    audio_pkt_bytes_accum = 0;
                                    audio_pkt_window_start = None;
                                    if let Some(ref v_rx) = video_packet_rx {
                                        while v_rx.try_recv().is_ok() {}
                                    }

                                    let (seek_stream_idx, target_pts) =
                                        if let (Some(v_idx), Some(tb)) =
                                            (video_stream_index, video_time_base)
                                        {
                                            let tb_f64 = tb.numerator() as f64
                                                / tb.denominator().max(1) as f64;
                                            (v_idx as i32, (pos / tb_f64).max(0.0) as i64)
                                        } else if stream_time_base > 0.0 {
                                            (
                                                best_audio_index as i32,
                                                (pos / stream_time_base).max(0.0) as i64,
                                            )
                                        } else {
                                            (
                                                -1,
                                                (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64)
                                                    .max(0.0)
                                                    as i64,
                                            )
                                        };
                                    unsafe {
                                        let ret = ffmpeg_next::ffi::av_seek_frame(
                                            ictx.as_mut_ptr(),
                                            seek_stream_idx,
                                            target_pts,
                                            ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                                        );
                                        if ret < 0 {
                                            let fallback_pts = (pos
                                                * ffmpeg_next::ffi::AV_TIME_BASE as f64)
                                                .max(0.0)
                                                as i64;
                                            ffmpeg_next::ffi::av_seek_frame(
                                                ictx.as_mut_ptr(),
                                                -1,
                                                fallback_pts,
                                                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                                            );
                                        }
                                    }
                                    let _ = decoder.flush();
                                    for acc in &mut accumulator {
                                        acc.fill(0.0);
                                    }
                                    // Wait briefly for feeder_thread to drain any pre-seek SPDIF bytes
                                    // buffered in the OS named pipe before advancing writer_epoch.
                                    let flush_wait_start = std::time::Instant::now();
                                    while pipe_flushing_worker.load(Ordering::SeqCst)
                                        && !stop_token_worker.load(Ordering::Relaxed)
                                        && flush_wait_start.elapsed()
                                            < std::time::Duration::from_millis(50)
                                    {
                                        std::thread::sleep(std::time::Duration::from_micros(500));
                                    }
                                    pipe_flushing_worker.store(false, Ordering::SeqCst);
                                    writer_epoch_worker.store(epoch, Ordering::SeqCst);
                                    current_seconds = pos;
                                }

                                if is_eof
                                    || (target_seek_time.is_none()
                                        && (pcm_tx_worker.len() > 256
                                            || (current_seconds > playback_sec + 7.5
                                                && pcm_tx_worker.len() > 16)))
                                {
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                    continue;
                                }

                                let Some((stream, mut packet)) = ictx.packets().next() else {
                                    is_eof = true;
                                    eof_flag_worker.store(true, Ordering::SeqCst);
                                    continue;
                                };

                                if Some(stream.index()) == video_stream_index {
                                    if let Some(ref v_tx) = video_packet_tx {
                                        let _ = v_tx.try_send((current_seek_epoch, packet.clone()));
                                    }
                                    continue;
                                }
                                if stream.index() == best_audio_index {
                                    let vis_packet = packet.clone();
                                    audio_pkt_bytes_accum =
                                        audio_pkt_bytes_accum.saturating_add(packet.size() as u64);

                                    let pts = packet.pts().or_else(|| packet.dts());
                                    if let Some(p) = pts {
                                        let pkt_sec = p as f64 * f64::from(stream.time_base());
                                        if seek_in_progress
                                            || (pkt_sec - current_seconds).abs() > 0.25
                                        {
                                            current_seconds = pkt_sec;
                                            seek_in_progress = false;
                                        }
                                        if let Some(start_t) = audio_pkt_window_start {
                                            let win_dur = pkt_sec - start_t;
                                            if win_dur >= 0.5 {
                                                let kbps = ((audio_pkt_bytes_accum as f64 * 8.0)
                                                    / (win_dur * 1000.0))
                                                    .round()
                                                    as u32;
                                                if kbps > 0
                                                    && let Ok(mut state) =
                                                        shared_state_worker.try_lock()
                                                    && state.stats.audio_bitrate_kbps.is_none()
                                                {
                                                    state.bitrate = Some(kbps);
                                                    state.stats.audio_bitrate_kbps = Some(kbps);
                                                }
                                                audio_pkt_window_start = Some(pkt_sec);
                                                audio_pkt_bytes_accum = 0;
                                            }
                                        } else {
                                            audio_pkt_window_start = Some(pkt_sec);
                                            audio_pkt_bytes_accum = 0;
                                        }
                                    }

                                    let mut skip_spdif_packet = false;
                                    if decoder.send_packet(&vis_packet).is_ok() {
                                        let mut frame = ffmpeg_next::frame::Audio::empty();
                                        while decoder.receive_frame(&mut frame).is_ok() {
                                            if stop_token_worker
                                                .load(std::sync::atomic::Ordering::Relaxed)
                                            {
                                                break;
                                            }
                                            let frame_layout = if frame.channel_layout().channels()
                                                > 0
                                            {
                                                frame.channel_layout()
                                            } else {
                                                ffmpeg_next::channel_layout::ChannelLayout::default(
                                                    frame.channels().max(1) as i32,
                                                )
                                            };

                                            if resampler.is_none()
                                                || frame.format() != cur_in_fmt
                                                || frame_layout != cur_in_layout
                                                || frame.rate() != cur_in_rate
                                            {
                                                resampler = ffmpeg_next::software::resampling::context::Context::get(
                                                    frame.format(),
                                                    frame_layout,
                                                    frame.rate(),
                                                    ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
                                                    target_channel_layout,
                                                    frame.rate(),
                                                ).ok();
                                                cur_in_fmt = frame.format();
                                                cur_in_layout = frame_layout;
                                                cur_in_rate = frame.rate();
                                            }

                                            if let Some(ref mut resamp) = resampler {
                                                let mut vis_frame =
                                                    ffmpeg_next::frame::Audio::empty();
                                                if resamp.run(&frame, &mut vis_frame).is_ok() {
                                                    let total_samples = vis_frame.samples();
                                                    let planes = (vis_frame.channels() as usize)
                                                        .min(target_ch);
                                                    if total_samples == 0 || planes == 0 {
                                                        continue;
                                                    }
                                                    let rate_u32 = decoder.rate().max(1);
                                                    let frame_dur =
                                                        total_samples as f64 / rate_u32 as f64;
                                                    if let Some(target_pos) = target_seek_time {
                                                        if current_seconds + frame_dur <= target_pos
                                                        {
                                                            current_seconds += frame_dur;
                                                            skip_spdif_packet = true;
                                                            continue;
                                                        } else {
                                                            if current_seconds < target_pos {
                                                                current_seconds = target_pos;
                                                            }
                                                            target_seek_time = None;
                                                            skip_spdif_packet = false;
                                                        }
                                                    }

                                                    let mut fresh_planes =
                                                        Vec::with_capacity(planes);
                                                    for p in 0..planes {
                                                        fresh_planes.push(
                                                            vis_frame.plane::<f32>(p)
                                                                [..total_samples]
                                                                .to_vec(),
                                                        );
                                                    }
                                                    if let Ok(mut state) =
                                                        shared_state_worker.lock()
                                                        && state.seek_epoch == current_seek_epoch
                                                    {
                                                        crate::audio::push_planar_lookahead_slices(
                                                            &mut state,
                                                            &fresh_planes,
                                                            rate_u32,
                                                            current_seconds,
                                                        );
                                                    }

                                                    let mut sample_offset = 0;
                                                    while sample_offset < total_samples {
                                                        let step = (total_samples - sample_offset)
                                                            .min(update_interval);
                                                        for (p, acc) in accumulator
                                                            .iter_mut()
                                                            .enumerate()
                                                            .take(planes)
                                                        {
                                                            let plane_data =
                                                                vis_frame.plane::<f32>(p);
                                                            acc.extend_from_slice(
                                                                &plane_data[sample_offset
                                                                    ..sample_offset + step],
                                                            );
                                                            let excess = acc
                                                                .len()
                                                                .saturating_sub(window_size);
                                                            if excess > 0 {
                                                                acc.drain(0..excess);
                                                            }
                                                        }

                                                        sample_offset += step;
                                                        samples_since_last_send += step;
                                                        if samples_since_last_send < update_interval
                                                        {
                                                            continue;
                                                        }
                                                        samples_since_last_send = 0;

                                                        let slice_time = current_seconds
                                                            + (sample_offset as f64
                                                                / rate_u32 as f64);

                                                        let mut channel_audio_data =
                                                            Vec::with_capacity(target_ch);
                                                        let mut channel_vus =
                                                            Vec::with_capacity(target_ch);
                                                        let eval_len = step.max(update_interval);
                                                        for acc in
                                                            accumulator.iter().take(target_ch)
                                                        {
                                                            let mut peak = 0.0f32;
                                                            let start_idx =
                                                                acc.len().saturating_sub(eval_len);
                                                            for &s in &acc[start_idx..] {
                                                                peak = peak.max(s.abs());
                                                            }
                                                            channel_vus.push(peak.clamp(0.0, 1.0));
                                                            channel_audio_data.push(acc.clone());
                                                        }

                                                        let mono_audio_data = {
                                                            let mut mono =
                                                                vec![0.0f32; window_size];
                                                            for (ch_idx, acc) in accumulator
                                                                .iter()
                                                                .take(target_ch)
                                                                .enumerate()
                                                            {
                                                                let ch_weight = match ch_idx {
                                                                    0 | 1 => {
                                                                        if target_ch == 1 {
                                                                            1.0
                                                                        } else {
                                                                            0.5
                                                                        }
                                                                    }
                                                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                                                    _ => 0.25,
                                                                };
                                                                for (i, &sample) in acc
                                                                    .iter()
                                                                    .take(window_size)
                                                                    .enumerate()
                                                                {
                                                                    mono[i] += sample * ch_weight;
                                                                }
                                                            }
                                                            for s in &mut mono {
                                                                *s = s.clamp(-1.0, 1.0);
                                                            }
                                                            mono
                                                        };
                                                        let vis_msg = DspMessage {
                                                            audio_data: mono_audio_data,
                                                            channel_vus,
                                                            current_order: 0,
                                                            current_row: 0,
                                                            bpm: 0,
                                                            speed: 0,
                                                            current_seconds: slice_time,
                                                            current_row_string: String::new(),
                                                            channel_audio_data,
                                                            seek_epoch: current_seek_epoch,
                                                        };
                                                        let _ = vis_tx_worker.try_send(vis_msg);
                                                    }
                                                    current_seconds += frame_dur;
                                                }
                                            }
                                        }
                                    }

                                    if !skip_spdif_packet {
                                        packet.set_pts(None);
                                        packet.set_dts(None);
                                        packet.set_position(-1);
                                        packet.set_stream(ost_index);

                                        if let Err(e) = packet.write(&mut octx) {
                                            eprintln!(
                                                "[bitstream] Failed to write packet to spdif: {}",
                                                e
                                            );
                                            break;
                                        }
                                    }
                                }
                            }
                            if !stop_token_worker.load(std::sync::atomic::Ordering::Relaxed) {
                                let _ = octx.write_trailer();
                            }
                        });

                        println!("[bitstream] Connecting named pipe for bitstream...");
                        unsafe {
                            let _ = ConnectNamedPipe(pipe_handle, None);
                        }
                        println!("[bitstream] Named pipe connected!");

                        let pipe_handle_raw = pipe_handle.0 as isize;
                        let pcm_tx_feeder = pcm_tx.clone();
                        let stop_token_feeder = stop_token_pump.clone();
                        let shared_state_feeder = shared_state_pump.clone();
                        let feeder_thread = std::thread::spawn(move || {
                            use std::os::windows::io::FromRawHandle;
                            let raw_ptr = pipe_handle_raw as *mut std::ffi::c_void;
                            let pipe_h = windows::Win32::Foundation::HANDLE(raw_ptr);
                            let mut file = unsafe { std::fs::File::from_raw_handle(raw_ptr) };
                            let mut buf = vec![0u8; 16384];
                            while !stop_token_feeder.load(Ordering::Relaxed) {
                                let mut avail: u32 = 0;
                                let peek_ok = unsafe {
                                    PeekNamedPipe(pipe_h, None, 0, None, Some(&mut avail), None)
                                };
                                if peek_ok.is_err() {
                                    pipe_flushing_feeder.store(false, Ordering::SeqCst);
                                    break;
                                }
                                if pipe_flushing_feeder.load(Ordering::SeqCst) {
                                    while avail > 0 {
                                        let to_drain = (avail as usize).min(buf.len());
                                        let _ = file.read(&mut buf[..to_drain]);
                                        avail = 0;
                                        if unsafe {
                                            PeekNamedPipe(
                                                pipe_h,
                                                None,
                                                0,
                                                None,
                                                Some(&mut avail),
                                                None,
                                            )
                                        }
                                        .is_err()
                                        {
                                            break;
                                        }
                                    }
                                    pipe_flushing_feeder.store(false, Ordering::SeqCst);
                                    continue;
                                }
                                if avail == 0 {
                                    if eof_flag_feeder.swap(false, Ordering::SeqCst) {
                                        let cur_epoch = writer_epoch_feeder.load(Ordering::SeqCst);
                                        let _ = pcm_tx_feeder.try_send(AudioPacket {
                                            pcm_bytes: Vec::new(),
                                            epoch: cur_epoch,
                                            is_eof: true,
                                        });
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(2));
                                    continue;
                                }
                                let chunk_epoch = writer_epoch_feeder.load(Ordering::SeqCst);
                                let to_read = (avail as usize).min(buf.len());
                                match file.read(&mut buf[..to_read]) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if pipe_flushing_feeder.load(Ordering::SeqCst) {
                                            continue;
                                        }
                                        let pkt = AudioPacket {
                                            pcm_bytes: buf[..n].to_vec(),
                                            epoch: chunk_epoch,
                                            is_eof: false,
                                        };
                                        let mut pending = Some(pkt);
                                        while let Some(chunk) = pending.take() {
                                            if stop_token_feeder
                                                .load(std::sync::atomic::Ordering::Relaxed)
                                            {
                                                drop(file);
                                                return;
                                            }
                                            if let Ok(state) = shared_state_feeder.try_lock()
                                                && (state.seek_request.is_some()
                                                    || state.seek_epoch > chunk.epoch)
                                            {
                                                break;
                                            }
                                            match pcm_tx_feeder.send_timeout(chunk, std::time::Duration::from_millis(20)) {
                                                Ok(()) => break,
                                                Err(crossbeam_channel::SendTimeoutError::Timeout(c)) => {
                                                    pending = Some(c);
                                                }
                                                Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                                                    drop(file);
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            drop(file);
                            if !stop_token_feeder.load(std::sync::atomic::Ordering::Relaxed) {
                                let final_epoch = shared_state_feeder
                                    .lock()
                                    .map(|s| s.seek_epoch)
                                    .unwrap_or(0);
                                let _ = pcm_tx_feeder.try_send(AudioPacket {
                                    pcm_bytes: Vec::new(),
                                    epoch: final_epoch,
                                    is_eof: true,
                                });
                            }
                        });

                        std::thread::spawn(move || {
                            let _ = feeder_thread.join();
                            let _ = ffmpeg_worker.join();
                        })
                    }
                    WasapiStreamType::Lpcm => {
                        let pcm_tx_lpcm = pcm_tx.clone();
                        let vis_tx_lpcm = vis_tx.clone();
                        let stop_token_lpcm = stop_token_ffmpeg.clone();
                        let shared_state_lpcm = shared_state_pump.clone();

                        std::thread::spawn(move || {
                            println!(
                                "[bitstream] Multi-Channel LPCM FFmpeg worker thread started."
                            );

                            let decoder_context =
                                match ffmpeg_next::codec::context::Context::from_parameters(
                                    parameters.clone(),
                                ) {
                                    Ok(ctx) => ctx,
                                    Err(e) => {
                                        eprintln!(
                                            "[bitstream] Failed to create LPCM decoder context: {}",
                                            e
                                        );
                                        return;
                                    }
                                };
                            let mut decoder = match decoder_context.decoder().audio() {
                                Ok(dec) => dec,
                                Err(e) => {
                                    eprintln!(
                                        "[bitstream] Failed to create LPCM audio decoder: {}",
                                        e
                                    );
                                    return;
                                }
                            };

                            let target_channel_layout =
                                ffmpeg_next::channel_layout::ChannelLayout::default(
                                    profile_clone.channels as i32,
                                );
                            let target_channels = profile_clone.channels as usize;
                            let bytes_per_sample = (profile_clone.container_bits / 8) as usize;
                            let out_rate = profile_clone.rate as f32;
                            let window_size = crate::audio::calculate_power_of_two_window_size(
                                profile_clone.rate,
                            );
                            let update_interval = ((out_rate / 60.0).round() as usize).max(256);
                            let mut accumulator: Vec<Vec<f32>> =
                                vec![vec![0.0f32; window_size]; target_channels];
                            let mut samples_since_last_send = 0usize;
                            let mut current_seconds = 0.0;

                            let mut pcm_resampler: Option<
                                ffmpeg_next::software::resampling::context::Context,
                            > = None;
                            let mut vis_resampler: Option<
                                ffmpeg_next::software::resampling::context::Context,
                            > = None;
                            let mut cur_in_fmt = ffmpeg_next::format::sample::Sample::None;
                            let mut cur_in_layout =
                                ffmpeg_next::channel_layout::ChannelLayout::default(0);
                            let mut cur_in_rate = 0;

                            let mut process_frame =
                                |frame: &ffmpeg_next::frame::Audio,
                                 current_seconds: &mut f64,
                                 target_seek_time: &mut Option<f64>,
                                 samples_since_last_send: &mut usize,
                                 epoch: u64,
                                 accumulator: &mut [Vec<f32>]| {
                                    let frame_layout = if frame.channel_layout().channels() > 0 {
                                        frame.channel_layout()
                                    } else {
                                        ffmpeg_next::channel_layout::ChannelLayout::default(
                                            frame.channels().max(1) as i32,
                                        )
                                    };

                                    if pcm_resampler.is_none()
                                        || frame.format() != cur_in_fmt
                                        || frame_layout != cur_in_layout
                                        || frame.rate() != cur_in_rate
                                    {
                                        pcm_resampler = ffmpeg_next::software::resampling::context::Context::get(
                                        frame.format(),
                                        frame_layout,
                                        frame.rate(),
                                        profile_clone.sample_format,
                                        target_channel_layout,
                                        profile_clone.rate,
                                    ).ok();
                                        vis_resampler = ffmpeg_next::software::resampling::context::Context::get(
                                        frame.format(),
                                        frame_layout,
                                        frame.rate(),
                                        ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
                                        target_channel_layout,
                                        profile_clone.rate,
                                    ).ok();
                                        cur_in_fmt = frame.format();
                                        cur_in_layout = frame_layout;
                                        cur_in_rate = frame.rate();
                                    }

                                    let rate_f64 = profile_clone.rate.max(1) as f64;
                                    let est_samples = frame.samples();
                                    let est_dur = est_samples as f64 / frame.rate().max(1) as f64;
                                    if let Some(target_pos) = *target_seek_time {
                                        if *current_seconds + est_dur <= target_pos {
                                            *current_seconds += est_dur;
                                            return;
                                        } else {
                                            if *current_seconds < target_pos {
                                                *current_seconds = target_pos;
                                            }
                                            *target_seek_time = None;
                                        }
                                    }

                                    if let Some(ref mut presamp) = pcm_resampler {
                                        let mut pcm_frame = ffmpeg_next::frame::Audio::empty();
                                        if presamp.run(&frame, &mut pcm_frame).is_ok() {
                                            let samples = pcm_frame.samples();
                                            let raw_bytes = pcm_frame.data(0);
                                            let expected_bytes =
                                                samples * target_channels * bytes_per_sample;
                                            if raw_bytes.len() >= expected_bytes {
                                                let pkt = AudioPacket {
                                                    pcm_bytes: raw_bytes[..expected_bytes].to_vec(),
                                                    epoch,
                                                    is_eof: false,
                                                };
                                                let mut pending = Some(pkt);
                                                while let Some(chunk) = pending.take() {
                                                    if stop_token_lpcm
                                                        .load(std::sync::atomic::Ordering::Relaxed)
                                                    {
                                                        return;
                                                    }
                                                    if let Ok(state) = shared_state_lpcm.try_lock()
                                                        && (state.seek_request.is_some()
                                                            || state.seek_epoch > chunk.epoch)
                                                    {
                                                        return;
                                                    }
                                                    match pcm_tx_lpcm.send_timeout(chunk, std::time::Duration::from_millis(20)) {
                                                    Ok(()) => break,
                                                    Err(crossbeam_channel::SendTimeoutError::Timeout(c)) => {
                                                        pending = Some(c);
                                                    }
                                                    Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                                                        return;
                                                    }
                                                }
                                                }
                                            }
                                        }
                                    }

                                    if let Some(ref mut vresamp) = vis_resampler {
                                        let mut vis_frame = ffmpeg_next::frame::Audio::empty();
                                        if vresamp.run(&frame, &mut vis_frame).is_ok() {
                                            let total_samples = vis_frame.samples();
                                            let planes = (vis_frame.channels() as usize)
                                                .min(target_channels);
                                            if total_samples == 0 || planes == 0 {
                                                return;
                                            }
                                            let mut fresh_planes = Vec::with_capacity(planes);
                                            for p in 0..planes {
                                                fresh_planes.push(
                                                    vis_frame.plane::<f32>(p)[..total_samples]
                                                        .to_vec(),
                                                );
                                            }
                                            if let Ok(mut state) = shared_state_lpcm.lock()
                                                && state.seek_epoch == epoch
                                            {
                                                crate::audio::push_planar_lookahead_slices(
                                                    &mut state,
                                                    &fresh_planes,
                                                    profile_clone.rate,
                                                    *current_seconds,
                                                );
                                            }

                                            let mut sample_offset = 0;

                                            while sample_offset < total_samples {
                                                let step = (total_samples - sample_offset)
                                                    .min(update_interval);
                                                for (p, acc) in
                                                    accumulator.iter_mut().enumerate().take(planes)
                                                {
                                                    let plane_data = vis_frame.plane::<f32>(p);
                                                    acc.extend_from_slice(
                                                        &plane_data
                                                            [sample_offset..sample_offset + step],
                                                    );
                                                    let excess =
                                                        acc.len().saturating_sub(window_size);
                                                    if excess > 0 {
                                                        acc.drain(0..excess);
                                                    }
                                                }

                                                sample_offset += step;
                                                *samples_since_last_send += step;
                                                if *samples_since_last_send < update_interval {
                                                    continue;
                                                }
                                                *samples_since_last_send = 0;

                                                let slice_time = *current_seconds
                                                    + (sample_offset as f64 / rate_f64);

                                                let mut channel_audio_data =
                                                    Vec::with_capacity(target_channels);
                                                let mut channel_vus =
                                                    Vec::with_capacity(target_channels);
                                                let eval_len = step.max(update_interval);
                                                for acc in accumulator.iter().take(target_channels)
                                                {
                                                    let mut peak = 0.0f32;
                                                    let start_idx =
                                                        acc.len().saturating_sub(eval_len);
                                                    for &s in &acc[start_idx..] {
                                                        peak = peak.max(s.abs());
                                                    }
                                                    channel_vus.push(peak.clamp(0.0, 1.0));
                                                    channel_audio_data.push(acc.clone());
                                                }

                                                let mono_audio_data = {
                                                    let mut mono = vec![0.0f32; window_size];
                                                    for (ch_idx, acc) in accumulator
                                                        .iter()
                                                        .take(target_channels)
                                                        .enumerate()
                                                    {
                                                        let ch_weight = match ch_idx {
                                                            0 | 1 => {
                                                                if target_channels == 1 {
                                                                    1.0
                                                                } else {
                                                                    0.5
                                                                }
                                                            }
                                                            2 => std::f32::consts::FRAC_1_SQRT_2,
                                                            _ => 0.25,
                                                        };
                                                        for (i, &sample) in
                                                            acc.iter().take(window_size).enumerate()
                                                        {
                                                            mono[i] += sample * ch_weight;
                                                        }
                                                    }
                                                    for s in &mut mono {
                                                        *s = s.clamp(-1.0, 1.0);
                                                    }
                                                    mono
                                                };
                                                let vis_msg = DspMessage {
                                                    audio_data: mono_audio_data,
                                                    channel_vus,
                                                    current_order: 0,
                                                    current_row: 0,
                                                    bpm: 0,
                                                    speed: 0,
                                                    current_seconds: slice_time,
                                                    current_row_string: String::new(),
                                                    channel_audio_data,
                                                    seek_epoch: epoch,
                                                };
                                                let _ = vis_tx_lpcm.try_send(vis_msg);
                                            }
                                            *current_seconds += total_samples as f64 / rate_f64;
                                        }
                                    }
                                };

                            let mut current_seek_epoch =
                                shared_state_lpcm.lock().map(|s| s.seek_epoch).unwrap_or(0);
                            let mut is_eof = false;
                            let mut seek_in_progress = true;
                            let mut target_seek_time: Option<f64> = None;
                            let mut audio_pkt_bytes_accum: u64 = 0;
                            let mut audio_pkt_window_start: Option<f64> = None;

                            loop {
                                if stop_token_lpcm.load(std::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }

                                let (seek_req, playback_sec) = {
                                    if let Ok(mut state) = shared_state_lpcm.lock() {
                                        if let Some(target) = state.seek_request.take() {
                                            state.seek_epoch += 1;
                                            state.current_seconds = target;
                                            state.track_ended = false;
                                            state.lookahead_queue.clear();
                                            state.lookahead_sample_buffer.clear();
                                            state.lookahead_timeline.fill(0.0);
                                            state.lookahead_buffer_start_time = target;
                                            state.waveform_history.clear();
                                            state.raw_waveform.fill(0.0);
                                            (Some((target, state.seek_epoch)), target)
                                        } else {
                                            (None, state.current_seconds)
                                        }
                                    } else {
                                        (None, current_seconds)
                                    }
                                };

                                if let Some((pos, epoch)) = seek_req {
                                    current_seek_epoch = epoch;
                                    is_eof = false;
                                    seek_in_progress = true;
                                    target_seek_time = Some(pos);
                                    samples_since_last_send = 0;
                                    audio_pkt_bytes_accum = 0;
                                    audio_pkt_window_start = None;
                                    if let Some(ref v_rx) = video_packet_rx {
                                        while v_rx.try_recv().is_ok() {}
                                    }

                                    let (seek_stream_idx, target_pts) =
                                        if let (Some(v_idx), Some(tb)) =
                                            (video_stream_index, video_time_base)
                                        {
                                            let tb_f64 = tb.numerator() as f64
                                                / tb.denominator().max(1) as f64;
                                            (v_idx as i32, (pos / tb_f64).max(0.0) as i64)
                                        } else if stream_time_base > 0.0 {
                                            (
                                                best_audio_index as i32,
                                                (pos / stream_time_base).max(0.0) as i64,
                                            )
                                        } else {
                                            (
                                                -1,
                                                (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64)
                                                    .max(0.0)
                                                    as i64,
                                            )
                                        };
                                    unsafe {
                                        let ret = ffmpeg_next::ffi::av_seek_frame(
                                            ictx.as_mut_ptr(),
                                            seek_stream_idx,
                                            target_pts,
                                            ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                                        );
                                        if ret < 0 {
                                            let fallback_pts = (pos
                                                * ffmpeg_next::ffi::AV_TIME_BASE as f64)
                                                .max(0.0)
                                                as i64;
                                            ffmpeg_next::ffi::av_seek_frame(
                                                ictx.as_mut_ptr(),
                                                -1,
                                                fallback_pts,
                                                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                                            );
                                        }
                                    }
                                    let _ = decoder.flush();
                                    for acc in &mut accumulator {
                                        acc.fill(0.0);
                                    }
                                    current_seconds = pos;
                                }

                                if is_eof
                                    || (target_seek_time.is_none()
                                        && (pcm_tx_lpcm.len() > 256
                                            || (current_seconds > playback_sec + 7.5
                                                && pcm_tx_lpcm.len() > 16)))
                                {
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                    continue;
                                }

                                if stop_token_lpcm.load(std::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }

                                match ictx.packets().next() {
                                    Some((stream, packet)) => {
                                        if Some(stream.index()) == video_stream_index {
                                            if let Some(ref v_tx) = video_packet_tx {
                                                let _ = v_tx
                                                    .try_send((current_seek_epoch, packet.clone()));
                                            }
                                        } else if stream.index() == best_audio_index {
                                            audio_pkt_bytes_accum = audio_pkt_bytes_accum
                                                .saturating_add(packet.size() as u64);
                                            let pts = packet.pts().or_else(|| packet.dts());
                                            if let Some(p) = pts {
                                                let pkt_sec = p as f64 * stream_time_base;
                                                if seek_in_progress
                                                    || (pkt_sec - current_seconds).abs() > 0.25
                                                {
                                                    current_seconds = pkt_sec;
                                                    seek_in_progress = false;
                                                }
                                                if let Some(start_t) = audio_pkt_window_start {
                                                    let win_dur = pkt_sec - start_t;
                                                    if win_dur >= 0.5 {
                                                        let kbps = ((audio_pkt_bytes_accum as f64
                                                            * 8.0)
                                                            / (win_dur * 1000.0))
                                                            .round()
                                                            as u32;
                                                        if kbps > 0
                                                            && let Ok(mut state) =
                                                                shared_state_lpcm.try_lock()
                                                            && state
                                                                .stats
                                                                .audio_bitrate_kbps
                                                                .is_none()
                                                        {
                                                            state.bitrate = Some(kbps);
                                                            state.stats.audio_bitrate_kbps =
                                                                Some(kbps);
                                                        }
                                                        audio_pkt_window_start = Some(pkt_sec);
                                                        audio_pkt_bytes_accum = 0;
                                                    }
                                                } else {
                                                    audio_pkt_window_start = Some(pkt_sec);
                                                    audio_pkt_bytes_accum = 0;
                                                }
                                            }

                                            if decoder.send_packet(&packet).is_ok() {
                                                let mut frame = ffmpeg_next::frame::Audio::empty();
                                                while decoder.receive_frame(&mut frame).is_ok() {
                                                    if stop_token_lpcm
                                                        .load(std::sync::atomic::Ordering::Relaxed)
                                                    {
                                                        break;
                                                    }
                                                    if let Ok(state) = shared_state_lpcm.lock() {
                                                        if state.seek_request.is_some() {
                                                            break;
                                                        }
                                                    }
                                                    process_frame(
                                                        &frame,
                                                        &mut current_seconds,
                                                        &mut target_seek_time,
                                                        &mut samples_since_last_send,
                                                        current_seek_epoch,
                                                        &mut accumulator,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    None => {
                                        let _ = decoder.send_eof();
                                        let mut frame = ffmpeg_next::frame::Audio::empty();
                                        while decoder.receive_frame(&mut frame).is_ok() {
                                            if stop_token_lpcm
                                                .load(std::sync::atomic::Ordering::Relaxed)
                                            {
                                                break;
                                            }
                                            process_frame(
                                                &frame,
                                                &mut current_seconds,
                                                &mut target_seek_time,
                                                &mut samples_since_last_send,
                                                current_seek_epoch,
                                                &mut accumulator,
                                            );
                                        }
                                        let _ = pcm_tx_lpcm.try_send(AudioPacket {
                                            pcm_bytes: Vec::new(),
                                            epoch: current_seek_epoch,
                                            is_eof: true,
                                        });
                                        is_eof = true;
                                    }
                                }
                            }
                        })
                    }
                };

                drop(pcm_tx);
                drop(vis_tx);

                Ok((
                    device,
                    format,
                    audio_client,
                    render_client,
                    event,
                    ffmpeg_thread,
                    pcm_rx,
                    vis_rx,
                    profile,
                    buffer_frames,
                    frame_bytes,
                ))
            })();

            match setup_result {
                Ok((
                    device,
                    format,
                    audio_client,
                    render_client,
                    event,
                    ffmpeg_thread,
                    pcm_rx,
                    vis_rx,
                    profile,
                    buffer_frames,
                    frame_bytes,
                )) => {
                    let out_rate = if stream_type == WasapiStreamType::CompressedBitstream {
                        decoder_sample_rate
                    } else {
                        profile.rate
                    };
                    let out_channels = if stream_type == WasapiStreamType::CompressedBitstream {
                        src_channels.clamp(2, 8)
                    } else {
                        profile.channels
                    };

                    let _ = init_tx.send(Ok((
                        out_rate,
                        out_channels,
                        codec_name_pump,
                        has_video,
                        meta_artist_pump,
                        meta_duration_pump,
                    )));

                    println!(
                        "\n>> Output Active: {} -> {}ch x {}Hz",
                        profile.name, profile.channels, profile.rate
                    );

                    let mut active_client = Some((audio_client, render_client, event));
                    let mut available = buffer_frames;
                    let frame_size = frame_bytes as usize;
                    let mut bytes_needed = (available * frame_bytes) as usize;
                    let mut target_prebuffer = bytes_needed.max(4096);
                    let mut buffer_queue: std::collections::VecDeque<u8> =
                        std::collections::VecDeque::with_capacity(bytes_needed * 4);
                    let mut started = false;
                    let mut eof = false;
                    let mut local_epoch = 0u64;

                    let prebuffer_start = std::time::Instant::now();
                    while buffer_queue.len() < target_prebuffer && !eof {
                        if stop_token_pump.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        match pcm_rx.recv_timeout(std::time::Duration::from_millis(10)) {
                            Ok(packet) => {
                                if packet.is_eof {
                                    eof = true;
                                    break;
                                }
                                local_epoch = packet.epoch;
                                buffer_queue.extend(packet.pcm_bytes);
                            }
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                                eof = true;
                                break;
                            }
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                                if prebuffer_start.elapsed() > std::time::Duration::from_millis(100)
                                {
                                    break;
                                }
                            }
                        }
                    }

                    let mut buffer_duration = available as f64 / profile.rate as f64;
                    let mut playback_time: Option<f64> = None;
                    let mut pending_vis_msg: Option<DspMessage> = None;

                    if let Ok(msg) = vis_rx.try_recv() {
                        playback_time = Some(msg.current_seconds);
                        let _ = tx.try_send(msg);
                    }

                    loop {
                        if stop_token_pump.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }

                        let (is_paused, seek_from_state) = {
                            if let Ok(state) = shared_state_pump.lock() {
                                let seek = if state.seek_epoch > local_epoch {
                                    local_epoch = state.seek_epoch;
                                    Some(state.current_seconds)
                                } else {
                                    None
                                };
                                (state.is_paused, seek)
                            } else {
                                (false, None)
                            }
                        };

                        if seek_from_state.is_some() {
                            buffer_queue.clear();
                            eof = false;
                            while let Ok(pkt) = pcm_rx.try_recv() {
                                if pkt.epoch >= local_epoch {
                                    if pkt.is_eof {
                                        eof = true;
                                        break;
                                    }
                                    buffer_queue.extend(pkt.pcm_bytes);
                                }
                            }
                            while vis_rx.try_recv().is_ok() {}
                            unsafe {
                                if let Some((ref ac, _, _)) = active_client {
                                    let _ = ac.Stop();
                                    let _ = ac.Reset();
                                }
                            }
                            playback_time = None;
                            pending_vis_msg = None;
                            started = false;
                        }

                        if is_paused {
                            if playback_time.is_none() {
                                let first_msg = if let Some(msg) = pending_vis_msg.take() {
                                    Some(msg)
                                } else {
                                    vis_rx.try_recv().ok()
                                };
                                if let Some(msg) = first_msg {
                                    let t = msg.current_seconds;
                                    let _ = tx.try_send(msg);
                                    playback_time = Some(t);
                                }
                            }
                            if started
                                || prebuffer_start.elapsed() > std::time::Duration::from_millis(500)
                            {
                                if let Some((ac, rc, ev)) = active_client.take() {
                                    unsafe {
                                        let _ = ac.Stop();
                                        let _ = ac.Reset();
                                    }
                                    drop(rc);
                                    drop(ac);
                                    unsafe {
                                        let _ = CloseHandle(ev);
                                    }
                                    started = false;
                                    println!(
                                        "[bitstream] Paused: released WASAPI Exclusive audio device lock."
                                    );
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(15));
                            continue;
                        }

                        if active_client.is_none() {
                            match initialize_exclusive_client(&device, &profile, &format) {
                                Ok((ac, rc, ev, bf)) => {
                                    available = bf;
                                    bytes_needed = (available * frame_bytes) as usize;
                                    target_prebuffer = bytes_needed.max(4096);
                                    buffer_duration = available as f64 / profile.rate as f64;
                                    active_client = Some((ac, rc, ev));
                                    started = false;
                                    println!(
                                        "[bitstream] Resumed: re-acquired WASAPI Exclusive audio device lock."
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[bitstream] Waiting to re-acquire WASAPI Exclusive device: {:?}",
                                        e
                                    );
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                    continue;
                                }
                            }
                        }
                        let (audio_client, render_client, event) = active_client
                            .as_ref()
                            .map(|(ac, rc, ev)| (ac, rc, *ev))
                            .unwrap();

                        if started {
                            let wait_result = unsafe { WaitForSingleObject(event, 50) };
                            if wait_result.0 == 258
                            /* WAIT_TIMEOUT */
                            {
                                continue;
                            }
                            if wait_result.0 != WAIT_OBJECT_0 {
                                break;
                            }
                        }

                        if false && is_paused {
                            if playback_time.is_none() {
                                let first_msg = if let Some(msg) = pending_vis_msg.take() {
                                    Some(msg)
                                } else {
                                    vis_rx.try_recv().ok()
                                };
                                if let Some(msg) = first_msg {
                                    let t = msg.current_seconds;
                                    let _ = tx.try_send(msg);
                                    playback_time = Some(t);
                                }
                            }
                            if started {
                                unsafe {
                                    if let Ok(buf) = render_client.GetBuffer(available) {
                                        ptr::write_bytes(buf, 0, bytes_needed);
                                        let _ = render_client.ReleaseBuffer(
                                            available,
                                            AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
                                        );
                                    }
                                }
                            } else {
                                std::thread::sleep(std::time::Duration::from_millis(15));
                            }
                            continue;
                        }

                        while buffer_queue.len() < target_prebuffer && !eof {
                            match pcm_rx.try_recv() {
                                Ok(packet) => {
                                    if packet.epoch < local_epoch {
                                        continue;
                                    }
                                    if packet.epoch > local_epoch {
                                        local_epoch = packet.epoch;
                                        buffer_queue.clear();
                                        while vis_rx.try_recv().is_ok() {}
                                        unsafe {
                                            if let Some((ref ac, _, _)) = active_client {
                                                let _ = ac.Stop();
                                                let _ = ac.Reset();
                                            }
                                        }
                                        playback_time = None;
                                        pending_vis_msg = None;
                                        started = false;
                                        eof = false;
                                    }
                                    if packet.is_eof {
                                        eof = true;
                                        break;
                                    }
                                    buffer_queue.extend(packet.pcm_bytes);
                                }
                                Err(crossbeam_channel::TryRecvError::Empty) => {
                                    break;
                                }
                                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                                    eof = true;
                                    break;
                                }
                            }
                        }

                        if let Ok(mut state) = shared_state_pump.try_lock() {
                            let queue_pct = (buffer_queue.len() as f32
                                / target_prebuffer.max(1) as f32)
                                * 100.0;
                            let chan_pct = (pcm_rx.len() as f32 / 64.0) * 100.0;
                            state.stats.audio_buffer_fill_pct =
                                queue_pct.max(chan_pct).clamp(0.0, 100.0);
                        }

                        if eof && buffer_queue.is_empty() {
                            if let Some(msg) = pending_vis_msg.take() {
                                let _ = tx.try_send(msg);
                            }
                            while let Ok(msg) = vis_rx.try_recv() {
                                let _ = tx.try_send(msg);
                            }
                            if let Ok(mut state) = shared_state_pump.lock() {
                                state.track_ended = true;
                            }
                            break;
                        }

                        let aligned_available = (buffer_queue.len() / frame_size) * frame_size;
                        let to_copy = bytes_needed.min(aligned_available);

                        if !started && to_copy < bytes_needed && !eof {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }

                        unsafe {
                            match render_client.GetBuffer(available) {
                                Ok(buf) => {
                                    if to_copy > 0 {
                                        let (s1, s2) = buffer_queue.as_slices();
                                        if s1.len() >= to_copy {
                                            ptr::copy_nonoverlapping(s1.as_ptr(), buf, to_copy);
                                        } else {
                                            let l1 = s1.len();
                                            ptr::copy_nonoverlapping(s1.as_ptr(), buf, l1);
                                            ptr::copy_nonoverlapping(
                                                s2.as_ptr(),
                                                buf.add(l1),
                                                to_copy - l1,
                                            );
                                        }
                                        buffer_queue.drain(0..to_copy);
                                    }
                                    if to_copy < bytes_needed {
                                        ptr::write_bytes(
                                            buf.add(to_copy),
                                            0,
                                            bytes_needed - to_copy,
                                        );
                                    }
                                    let flags = if to_copy == 0 {
                                        AUDCLNT_BUFFERFLAGS_SILENT.0 as u32
                                    } else {
                                        0
                                    };
                                    if let Err(e) = render_client.ReleaseBuffer(available, flags) {
                                        eprintln!("[bitstream] ReleaseBuffer error: {:?}", e);
                                        break;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[bitstream] GetBuffer error: {:?}", e);
                                    break;
                                }
                            }
                        }

                        if !started {
                            unsafe {
                                if let Err(e) = audio_client.Start() {
                                    eprintln!("[bitstream] audio_client.Start error: {:?}", e);
                                    break;
                                }
                            }
                            started = true;
                        } else if let Some(pos) = playback_time.as_mut() {
                            *pos += buffer_duration;
                            let target_t = *pos;
                            loop {
                                let next_msg = if let Some(msg) = pending_vis_msg.take() {
                                    Some(msg)
                                } else {
                                    vis_rx.try_recv().ok()
                                };
                                let Some(msg) = next_msg else {
                                    break;
                                };
                                if msg.current_seconds <= target_t {
                                    let _ = tx.try_send(msg);
                                } else {
                                    pending_vis_msg = Some(msg);
                                    break;
                                }
                            }
                        } else {
                            let first_msg = if let Some(msg) = pending_vis_msg.take() {
                                Some(msg)
                            } else {
                                vis_rx.try_recv().ok()
                            };
                            if let Some(msg) = first_msg {
                                let t = msg.current_seconds;
                                let _ = tx.try_send(msg);
                                playback_time = Some(t + buffer_duration);
                            }
                        }
                    }

                    // Strict Teardown:
                    // 1. Stop audio hardware and reset (if not already released while paused)
                    if let Some((audio_client, render_client, event)) = active_client.take() {
                        unsafe {
                            let _ = audio_client.Stop();
                            let _ = audio_client.Reset();
                        }
                        // 2. Drop render and audio client COM interfaces BEFORE CoUninitialize!
                        drop(render_client);
                        drop(audio_client);

                        // 3. Close event handle
                        unsafe {
                            let _ = CloseHandle(event);
                        }
                    }
                    drop(device);

                    // 4. Signal stop and drop receivers so worker thread exits promptly
                    stop_token_pump.store(true, std::sync::atomic::Ordering::SeqCst);
                    drop(pcm_rx);
                    drop(vis_rx);

                    // 5. Join worker thread
                    let _ = ffmpeg_thread.join();

                    // 6. Uninitialize COM
                    unsafe {
                        CoUninitialize();
                    }

                    println!("[bitstream] Bitstream/LPCM pump thread finished cleanly.");
                }
                Err(e) => {
                    eprintln!("[bitstream] Initialization failed: {:?}", e);
                    let _ = init_tx.send(Err(e));
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        });

        // Caller waits for pump thread initialization
        match init_rx.recv_timeout(std::time::Duration::from_millis(3000)) {
            Ok(Ok((out_rate, out_channels, codec_name, has_video, artist, dur))) => Ok((
                handle,
                out_rate,
                out_channels,
                codec_name,
                has_video,
                artist,
                dur,
            )),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                stop_token.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = handle.join();
                Err(anyhow::anyhow!(
                    "Timed out initializing WASAPI Exclusive stream"
                ))
            }
        }
    }
}
