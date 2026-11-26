use super::*;
use crate::state::AppState;
use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn start_audio_thread(
    file_path: &str,
    mic: bool,
    shared_state: Arc<Mutex<AppState>>,
) -> Result<PlaybackHandle> {
    let mut preloaded_source: Option<Box<dyn AudioSource>> = None;
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if !mic {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let can_bitstream = matches!(
            ext.as_str(),
            "mkv"
                | "mp4"
                | "m2ts"
                | "ts"
                | "mts"
                | "mov"
                | "avi"
                | "mka"
                | "vob"
                | "aob"
                | "iso"
                | "dtshd"
                | "dts"
                | "ac3"
                | "eac3"
                | "thd"
                | "truehd"
                | "mlp"
        );
        let (passthrough, surround_upmix_enabled, force_stereo) = {
            let s = shared_state.lock().unwrap();
            (
                s.passthrough_enabled,
                s.surround_upmix.enabled,
                s.force_stereo_downmix,
            )
        };
        let should_try_bitstream = if passthrough
            && can_bitstream
            && !surround_upmix_enabled
            && !force_stereo
        {
            true
        } else if can_bitstream && !force_stereo {
            match load_audio_source(file_path) {
                Ok(src) => {
                    preloaded_source = Some(src);
                    false
                }
                Err(e) => {
                    eprintln!(
                        "[audio] Shared-mode software decode failed ({}), falling back to exclusive bitstream passthrough...",
                        e
                    );
                    true
                }
            }
        } else {
            false
        };
        if should_try_bitstream {
            {
                let mut state = shared_state.lock().unwrap();
                state.current_seconds = 0.0;
                state.lookahead_queue.clear();
                state.lookahead_sample_buffer.clear();
                state.lookahead_buffer_start_time = 0.0;
                state.lookahead_timeline = vec![0.0; 600 * 8];
                state.waveform_history.clear();
                state.raw_waveform.fill(0.0);
                state.track_ended = false;
            }
            let (tx, rx) = bounded::<DspMessage>(32);
            let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
            if let Ok((handle, decoder_rate, channels, codec_name, has_video, artist, duration)) =
                crate::bitstream::start_bitstream_thread(
                    file_path,
                    shared_state.clone(),
                    tx.clone(),
                    stop_token.clone(),
                )
            {
                let max_frequency = shared_state.lock().unwrap().max_frequency;
                let sample_rate = decoder_rate;
                let window_size = calculate_power_of_two_window_size(sample_rate);

                let is_lpcm = codec_name == "flac"
                    || codec_name == "pcm"
                    || codec_name == "alac"
                    || codec_name == "multichannel_pcm";
                let display_name = match codec_name.as_str() {
                    "truehd" => "TrueHD / Dolby Atmos",
                    "eac3" => "E-AC3 / Dolby Digital Plus",
                    "dts" => "DTS",
                    "ac3" => "AC3 / Dolby Digital",
                    "flac" => "FLAC Multi-Channel LPCM",
                    "pcm" => "Multi-Channel LPCM",
                    "alac" => "ALAC Multi-Channel LPCM",
                    _ => &codec_name,
                }
                .to_string();

                let video_suffix = if has_video {
                    " (Video available: 'v' to view)"
                } else {
                    ""
                };

                let default_artist = if is_lpcm {
                    "WASAPI Exclusive LPCM"
                } else {
                    "Bitstream Active"
                };
                let meta_artist = if !artist.is_empty() {
                    artist
                } else {
                    default_artist.to_string()
                };
                let meta_duration = duration;

                {
                    let mut state = shared_state.lock().unwrap();
                    state.artist = meta_artist;
                    state.module_type = if is_lpcm {
                        "Multi-Channel LPCM".to_string()
                    } else {
                        "Hardware Passthrough".to_string()
                    };
                    state.stats.bitstream_active = true;
                    state.stats.mpegh_bitstream_active = false;
                    state.stats.mpegh_carrier_info.clear();
                    state.is_dummy_audio_output = false;
                    state.current_sample_rate = sample_rate as f32;
                    state.duration_seconds = meta_duration;
                    let ch = (channels as i32).max(2);
                    state.num_channels = ch;
                    state.hardware_channels = ch;
                    state.channel_vus = vec![0.0; ch as usize];
                    state.peak_vus = vec![0.0; ch as usize];
                    state.has_video_stream = has_video;
                    state.video_info = Some(format!("{}{}", display_name, video_suffix));
                }

                spawn_dsp_thread(
                    rx,
                    shared_state.clone(),
                    sample_rate,
                    max_frequency,
                    window_size,
                );
                return Ok(PlaybackHandle::Bitstream(Some(handle), stop_token));
            }
        }
    }

    {
        let mut state = shared_state.lock().unwrap();
        state.stats.bitstream_active = false;
        state.stats.mpegh_bitstream_active = false;
        state.stats.mpegh_carrier_info.clear();
        state.is_dummy_audio_output = false;
        state.stats.audio_buffer_fill_pct = 0.0;
    }

    let host = cpal::default_host();
    let mut audio_source_opt = if mic {
        None
    } else if let Some(src) = preloaded_source.take() {
        Some(src)
    } else {
        Some(load_audio_source(file_path)?)
    };
    let rate = audio_source_opt
        .as_mut()
        .and_then(|a| a.get_intrinsic_sample_rate())
        .unwrap_or(48000);
    let target_rate: cpal::SampleRate = rate;
    let (
        force_stereo,
        surround_upmix_enabled,
        surround_upmix_user_overridden,
        passthrough_enabled,
        mpegh_bitstream,
        mpegh_user_overridden,
    ) = {
        let s = shared_state.lock().unwrap();
        (
            s.force_stereo_downmix,
            s.surround_upmix.enabled,
            s.surround_upmix.user_overridden,
            s.passthrough_enabled,
            s.surround_upmix.mpegh_bitstream,
            s.surround_upmix.mpegh_user_overridden,
        )
    };
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = (mpegh_bitstream, mpegh_user_overridden);
    let is_native_atmos = audio_source_opt
        .as_ref()
        .is_some_and(|a| a.is_native_atmos());
    let source_channels = audio_source_opt
        .as_mut()
        .map(|a| a.get_num_channels() as u16)
        .unwrap_or(2);
    let target_channels = if force_stereo { 2 } else { source_channels };

    if mic {
        let selected_device_name = {
            let state = shared_state.lock().unwrap();
            state.selected_audio_device.clone()
        };
        let mut devices_to_try: Vec<cpal::Device> = Vec::new();
        let push_unique_input_device = |list: &mut Vec<cpal::Device>, dev: cpal::Device| {
            let dev_id = dev.id().ok();
            let dev_name = dev
                .description()
                .ok()
                .map(|desc| {
                    format!(
                        "{}:{:?}:{:?}",
                        format_cpal_device_name(&desc),
                        desc.device_type(),
                        desc.address()
                    )
                })
                .unwrap_or_default();
            let already = list.iter().any(|existing| {
                if let (Some(id_a), Ok(id_b)) = (dev_id.as_ref(), existing.id()) {
                    return *id_a == id_b;
                }
                let ex_name = existing
                    .description()
                    .ok()
                    .map(|desc| {
                        format!(
                            "{}:{:?}:{:?}",
                            format_cpal_device_name(&desc),
                            desc.device_type(),
                            desc.address()
                        )
                    })
                    .unwrap_or_default();
                !dev_name.is_empty() && dev_name == ex_name
            });
            if !already {
                list.push(dev);
            }
        };

        let rank_input_device = |dev: &cpal::Device| -> u8 {
            let Ok(desc) = dev.description() else {
                return 2;
            };
            let is_mic_or_headset = matches!(
                desc.device_type(),
                cpal::DeviceType::Microphone
                    | cpal::DeviceType::Headset
                    | cpal::DeviceType::Headphones
            );
            let is_primary_addr = desc
                .address()
                .is_none_or(|a| a.eq_ignore_ascii_case("bottom") || a.is_empty());
            match (is_mic_or_headset, is_primary_addr) {
                (true, true) => 0,
                (true, false) => 1,
                (false, _) => 2,
            }
        };

        if !is_default_audio_device_selection(selected_device_name.as_deref())
            && let Some(ref name) = selected_device_name
        {
            if let Some(default_dev) = host.default_input_device()
                && let Ok(desc) = default_dev.description()
                && is_usable_cpal_device_for_mode(&desc, true)
                && cpal_device_matches_name(&desc, name)
            {
                push_unique_input_device(&mut devices_to_try, default_dev);
            }
            if let Ok(devices) = host.input_devices() {
                let mut matched: Vec<cpal::Device> = devices
                    .filter(|d| {
                        d.description().is_ok_and(|desc| {
                            is_usable_cpal_device_for_mode(&desc, true)
                                && cpal_device_matches_name(&desc, name)
                        })
                    })
                    .collect();
                matched.sort_by_key(&rank_input_device);
                for d in matched {
                    push_unique_input_device(&mut devices_to_try, d);
                }
            }
        }

        if let Some(default_dev) = host.default_input_device() {
            push_unique_input_device(&mut devices_to_try, default_dev);
        }
        if let Ok(devices) = host.input_devices() {
            let mut others: Vec<cpal::Device> = devices
                .filter(|d| {
                    d.description().is_ok_and(|desc| {
                        is_usable_cpal_device_for_mode(&desc, true)
                            && is_relevant_audio_device(&format_cpal_device_name(&desc))
                    })
                })
                .collect();
            others.sort_by_key(&rank_input_device);
            for d in others {
                push_unique_input_device(&mut devices_to_try, d);
            }
        }

        if devices_to_try.is_empty() {
            return Err(anyhow::anyhow!("No input device available"));
        }

        let mut last_mic_err = anyhow::anyhow!("No candidate input device succeeded");
        for device in devices_to_try {
            let actual_device_name = device
                .description()
                .ok()
                .map(|desc| format_cpal_device_name(&desc))
                .filter(|n| {
                    !n.eq_ignore_ascii_case("Default Device") && !n.eq_ignore_ascii_case("default")
                })
                .or_else(|| {
                    selected_device_name
                        .clone()
                        .filter(|n| !is_default_audio_device_selection(Some(n)))
                })
                .unwrap_or_else(|| "Microphone".to_string());

            let mut candidate_configs: Vec<cpal::SupportedStreamConfig> = Vec::new();
            if let Ok(supported_configs_range) = device.supported_input_configs() {
                let mut input_configs: Vec<_> = supported_configs_range
                    .filter(|c| {
                        c.sample_format() == cpal::SampleFormat::F32
                            || c.sample_format() == cpal::SampleFormat::I16
                    })
                    .collect();
                input_configs.sort_by_key(|c| {
                    let rate_match =
                        c.min_sample_rate() <= target_rate && c.max_sample_rate() >= target_rate;
                    let is_f32 = c.sample_format() == cpal::SampleFormat::F32;
                    let ch_pref = c.channels() == 2 || c.channels() == 1;
                    (
                        std::cmp::Reverse(rate_match),
                        std::cmp::Reverse(is_f32),
                        std::cmp::Reverse(ch_pref),
                        std::cmp::Reverse(c.channels()),
                    )
                });
                for c in input_configs {
                    let rate = if c.min_sample_rate() <= target_rate
                        && c.max_sample_rate() >= target_rate
                    {
                        target_rate
                    } else {
                        c.max_sample_rate()
                    };
                    let resolved = c.with_sample_rate(rate);
                    if !candidate_configs.iter().any(|existing| {
                        existing.channels() == resolved.channels()
                            && existing.sample_rate() == resolved.sample_rate()
                            && existing.sample_format() == resolved.sample_format()
                    }) {
                        candidate_configs.push(resolved);
                        if candidate_configs.len() >= 4 {
                            break;
                        }
                    }
                }
            }
            if let Ok(default_cfg) = device.default_input_config()
                && matches!(
                    default_cfg.sample_format(),
                    cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
                )
                && !candidate_configs.iter().any(|existing| {
                    existing.channels() == default_cfg.channels()
                        && existing.sample_rate() == default_cfg.sample_rate()
                        && existing.sample_format() == default_cfg.sample_format()
                })
            {
                candidate_configs.push(default_cfg);
            }

            if candidate_configs.is_empty() {
                last_mic_err = anyhow::anyhow!(
                    "No supported input config on device '{}'",
                    actual_device_name
                );
                continue;
            }

            for supported_config in candidate_configs {
                let config: cpal::StreamConfig = supported_config.into();
                let (tx, rx) = bounded::<DspMessage>(32);

                let stream_res = match supported_config.sample_format() {
                    cpal::SampleFormat::F32 => run_mic::<f32>(
                        &device,
                        &config,
                        shared_state.clone(),
                        tx,
                        rx.clone(),
                        config.sample_rate,
                    ),
                    cpal::SampleFormat::I16 => run_mic::<i16>(
                        &device,
                        &config,
                        shared_state.clone(),
                        tx,
                        rx.clone(),
                        config.sample_rate,
                    ),
                    cpal::SampleFormat::U16 => run_mic::<u16>(
                        &device,
                        &config,
                        shared_state.clone(),
                        tx,
                        rx.clone(),
                        config.sample_rate,
                    ),
                    _ => Err(anyhow::anyhow!("Unsupported sample format")),
                };

                match stream_res {
                    Ok((stream, stream_recycle_tx, keep_alive)) => {
                        let max_frequency = {
                            let mut state = shared_state.lock().unwrap();
                            state.artist = actual_device_name;
                            state.module_type = "Hardware Input".to_string();
                            state.duration_seconds = 0.0;
                            state.current_seconds = 0.0;
                            state.bitrate = None;
                            state.audio_tracks.clear();
                            state.selected_audio_track = 0;
                            state.active_audio_tracks.clear();
                            state.audio_track_volumes.clear();
                            state.tracker_channels = None;
                            state.tracker_patterns_by_order = std::sync::Arc::new(Vec::new());
                            state.tracker_row_history.clear();
                            state.current_tracker_row_string.clear();
                            state.num_samples = 0;
                            state.num_instruments = 0;
                            state.num_patterns = 0;
                            state.source_channels = config.channels as i32;
                            state.hardware_channels = config.channels as i32;
                            state.num_channels = config.channels as i32;
                            state.raw_channel_vus = vec![0.0; state.num_channels as usize];
                            state.channel_vus = vec![0.0; state.num_channels as usize];
                            state.peak_vus = vec![0.0; state.num_channels as usize];
                            state.bpm = 0;
                            state.speed = 0;
                            state.max_frequency = config.sample_rate as f32 / 2.0;
                            state.current_sample_rate = config.sample_rate as f32;
                            state.sync_rt_control();
                            state.max_frequency
                        };
                        let window_size = calculate_power_of_two_window_size(config.sample_rate);
                        spawn_dsp_thread_with_recycle(
                            rx,
                            shared_state,
                            config.sample_rate,
                            max_frequency,
                            window_size,
                            Some(stream_recycle_tx),
                        );
                        return Ok(PlaybackHandle::Cpal(
                            stream,
                            Arc::new(std::sync::atomic::AtomicBool::new(false)),
                            keep_alive,
                        ));
                    }
                    Err(e) => {
                        last_mic_err = e;
                    }
                }
            }
        }

        return Err(last_mic_err);
    }

    let shared_state_closure = shared_state.clone();
    let cpal_stream_result = (|| -> Result<PlaybackHandle> {
        let selected_device_name = {
            let state = shared_state_closure.lock().unwrap();
            state.selected_audio_device.clone()
        };

        let mut devices_to_try = Vec::new();
        let push_unique_device = |list: &mut Vec<cpal::Device>, dev: cpal::Device| {
            let d_name = dev
                .description()
                .map(|desc| format_cpal_device_name(&desc))
                .unwrap_or_else(|_| "Unknown Device".to_string());
            let already_present = list.iter().any(|existing| {
                existing
                    .description()
                    .map(|desc| format_cpal_device_name(&desc) == d_name)
                    .unwrap_or(false)
            });
            if !already_present {
                list.push(dev);
            }
        };

        if !is_default_audio_device_selection(selected_device_name.as_deref())
            && let Some(ref name) = selected_device_name
        {
            let mut is_default = false;

            // ALSA WORKAROUND: If the user selected the default device by name, ensure we use the REAL default host handle.
            // Matching by name in the output_devices() iterator on Linux can inadvertently pick a raw hardware endpoint
            // that bypasses PulseAudio/PipeWire and fails with ALSA `unable to open slave` lock errors.
            if let Some(default_dev) = host.default_output_device()
                && let Ok(desc) = default_dev.description()
                && cpal_device_matches_name(&desc, name)
            {
                push_unique_device(&mut devices_to_try, default_dev);
                is_default = true;
            }

            if !is_default && let Ok(devices) = host.output_devices() {
                for d in devices {
                    if let Ok(desc) = d.description()
                        && is_usable_cpal_device_for_mode(&desc, false)
                        && cpal_device_matches_name(&desc, name)
                    {
                        push_unique_device(&mut devices_to_try, d);
                        break;
                    }
                }
            }
        }

        if let Some(default_dev) = host.default_output_device() {
            let is_relevant = default_dev
                .description()
                .map(|desc| {
                    is_usable_cpal_device_for_mode(&desc, false)
                        && is_relevant_audio_device(&format_cpal_device_name(&desc))
                })
                .unwrap_or(true);
            if is_relevant {
                push_unique_device(&mut devices_to_try, default_dev);
            }
        }
        if let Ok(devices) = host.output_devices() {
            for d in devices {
                let is_relevant = d
                    .description()
                    .map(|desc| {
                        is_usable_cpal_device_for_mode(&desc, false)
                            && is_relevant_audio_device(&format_cpal_device_name(&desc))
                    })
                    .unwrap_or(true);
                if is_relevant {
                    push_unique_device(&mut devices_to_try, d);
                }
            }
        }

        if devices_to_try.is_empty() {
            return Err(anyhow::anyhow!("No audio output devices found."));
        }

        let mut last_err = anyhow::anyhow!("No candidate audio device succeeded");
        for (dev_idx, device) in devices_to_try.into_iter().enumerate() {
            let device_name = device
                .description()
                .map(|desc| format_cpal_device_name(&desc))
                .unwrap_or_else(|_| "Unknown Device".to_string());

            // Reuse initial audio source for the first attempt, or reload if retrying on fallback device
            let mut attempt_audio_source_opt = if mic {
                None
            } else if let Some(src) = audio_source_opt.take() {
                Some(src)
            } else if let Some(src) = preloaded_source.take() {
                Some(src)
            } else {
                Some(load_audio_source(file_path)?)
            };

            let result = (|| -> Result<PlaybackHandle> {
                let base_device_channels = device
                    .default_output_config()
                    .ok()
                    .map(|c| c.channels())
                    .unwrap_or(2);
                #[cfg(target_os = "linux")]
                let default_device_channels = {
                    let is_default_cand = dev_idx == 0
                        && is_default_audio_device_selection(selected_device_name.as_deref());
                    let d_lower = device_name.to_lowercase();
                    if (is_default_cand
                        || d_lower == "default"
                        || d_lower.contains("pipewire")
                        || d_lower.contains("pulseaudio"))
                        && let Some(topo) = query_active_audio_sink_topology(false)
                    {
                        if let Ok(mut state) = shared_state_closure.lock() {
                            state.active_audio_sink_id = Some(topo.sink_id);
                            state.active_audio_sink_description = Some(topo.description);
                            state.active_audio_sink_channels = topo.channels;
                        }
                        topo.channels.max(1)
                    } else {
                        base_device_channels
                    }
                };
                #[cfg(not(target_os = "linux"))]
                let default_device_channels = base_device_channels;
                let effective_surround_upmix = if is_native_atmos {
                    false
                } else if surround_upmix_user_overridden {
                    surround_upmix_enabled
                } else {
                    !force_stereo
                        && !passthrough_enabled
                        && crate::state::SurroundUpmixConfig::should_enable_by_default(
                            source_channels as i32,
                            default_device_channels as i32,
                        )
                };

                // On Windows HDMI/DisplayPort/AVR endpoints where the Windows Shared Mode default
                // mix format is Stereo (default_device_channels <= 2), cpal only exposes
                // `GetMixFormat()` and AudioDg.exe downmixes any Shared Mode stream to 2.0 stereo.
                // Probe and open WASAPI Exclusive Mode multi-channel LPCM (up to 7.1 / 7.1.4) or
                // IEC 61937-13 MPEG-H 3D Audio 7.1.4 (11.1) directly on the endpoint so the HDMI
                // Audio InfoFrame switches to multi-channel surround.
                #[cfg(windows)]
                if !mic && !force_stereo {
                    let want_wasapi_upmix = if is_native_atmos {
                        false
                    } else if surround_upmix_user_overridden {
                        surround_upmix_enabled
                    } else {
                        true
                    };
                    let allow_exclusive = passthrough_enabled
                        || (surround_upmix_user_overridden && surround_upmix_enabled)
                        || mpegh_user_overridden
                        || target_channels > 2;
                    let wasapi_target_dev = if dev_idx == 0
                        && is_default_audio_device_selection(selected_device_name.as_deref())
                    {
                        None
                    } else {
                        Some(device_name.as_str())
                    };
                    match run_wasapi_exclusive_multichannel(
                        wasapi_target_dev,
                        &mut attempt_audio_source_opt,
                        shared_state_closure.clone(),
                        target_rate,
                        target_channels,
                        want_wasapi_upmix,
                        mpegh_bitstream,
                        allow_exclusive,
                        file_path,
                    ) {
                        Ok(handle) => {
                            if dev_idx > 0
                                && !is_default_audio_device_selection(
                                    selected_device_name.as_deref(),
                                )
                                && let Ok(mut state) = shared_state_closure.lock()
                            {
                                eprintln!(
                                    "[audio] Switched to available fallback audio device: '{}'",
                                    device_name
                                );
                                state.selected_audio_device = Some(device_name.clone());
                            }
                            return Ok(handle);
                        }
                        Err(e) => {
                            eprintln!(
                                "[audio] WASAPI Exclusive Multichannel probe on '{}': {:?} (falling back to CPAL)",
                                device_name, e
                            );
                        }
                    }
                }

                #[cfg(target_os = "linux")]
                if !mic
                    && !force_stereo
                    && mpegh_bitstream
                    && (effective_surround_upmix || target_channels >= 9)
                    && (mpegh_user_overridden
                        || (default_device_channels > 2
                            && default_device_channels < 12
                            && device_name.to_lowercase().contains("hdmi")))
                {
                    match run_linux_pipewire_mpegh_714(
                        &mut attempt_audio_source_opt,
                        shared_state_closure.clone(),
                        file_path,
                    ) {
                        Ok(handle) => {
                            return Ok(handle);
                        }
                        Err(e) => {
                            eprintln!(
                                "[audio] Linux PipeWire MPEG-H 3D Audio 7.1.4 probe on '{}': {:?} (falling back to CPAL)",
                                device_name, e
                            );
                        }
                    }
                }

                // Only request >2 output channels for surround upmix when the OS audio endpoint
                // is actually configured for multi-channel surround (>2ch). On standard stereo/mono
                // speakers (default_device_channels <= 2), surround upmix is bypassed completely
                // and hardware_channels stays at the native stereo/mono channel count.
                let upmix_to_multichannel_hw =
                    effective_surround_upmix && default_device_channels > 2;
                let device_target_channels = resolve_device_target_channels(
                    force_stereo,
                    default_device_channels,
                    target_channels,
                );

                let mut candidate_configs: Vec<cpal::SupportedStreamConfig> = Vec::new();
                if let Ok(supported_configs_range) = device.supported_output_configs() {
                    let mut configs: Vec<_> = supported_configs_range
                        .filter(|c| {
                            c.sample_format() == cpal::SampleFormat::F32
                                || c.sample_format() == cpal::SampleFormat::I16
                        })
                        .collect();

                    configs.sort_by_key(|c| {
                        rank_output_config_key(
                            c.channels(),
                            c.min_sample_rate(),
                            c.max_sample_rate(),
                            c.sample_format(),
                            device_target_channels,
                            target_rate,
                            upmix_to_multichannel_hw,
                            force_stereo,
                        )
                    });

                    for c in configs {
                        let rate = if c.min_sample_rate() <= target_rate
                            && c.max_sample_rate() >= target_rate
                        {
                            target_rate
                        } else {
                            c.max_sample_rate()
                        };
                        let resolved = c.with_sample_rate(rate);
                        if !candidate_configs.iter().any(|existing| {
                            existing.channels() == resolved.channels()
                                && existing.sample_rate() == resolved.sample_rate()
                                && existing.sample_format() == resolved.sample_format()
                        }) {
                            candidate_configs.push(resolved);
                            if candidate_configs.len() >= 4 {
                                break;
                            }
                        }
                    }
                }

                if let Ok(default_cfg) = device.default_output_config()
                    && matches!(
                        default_cfg.sample_format(),
                        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
                    )
                    && !candidate_configs.iter().any(|existing| {
                        existing.channels() == default_cfg.channels()
                            && existing.sample_rate() == default_cfg.sample_rate()
                            && existing.sample_format() == default_cfg.sample_format()
                    })
                {
                    candidate_configs.push(default_cfg);
                }

                if candidate_configs.is_empty() {
                    return Err(anyhow::anyhow!("No supported output config on device"));
                }

                let mut last_cfg_err = anyhow::anyhow!("No output config succeeded on device");
                for supported_config in candidate_configs {
                    let config: cpal::StreamConfig = supported_config.into();
                    let (tx, rx) = bounded::<DspMessage>(32);
                    let window_size = calculate_power_of_two_window_size(config.sample_rate);

                    let stream_res = match supported_config.sample_format() {
                        cpal::SampleFormat::F32 => run::<f32>(
                            &device,
                            &config,
                            &mut attempt_audio_source_opt,
                            shared_state_closure.clone(),
                            tx,
                            rx.clone(),
                            config.sample_rate,
                            0.0,
                            window_size,
                            file_path,
                        ),
                        cpal::SampleFormat::I16 => run::<i16>(
                            &device,
                            &config,
                            &mut attempt_audio_source_opt,
                            shared_state_closure.clone(),
                            tx,
                            rx.clone(),
                            config.sample_rate,
                            0.0,
                            window_size,
                            file_path,
                        ),
                        cpal::SampleFormat::U16 => run::<u16>(
                            &device,
                            &config,
                            &mut attempt_audio_source_opt,
                            shared_state_closure.clone(),
                            tx,
                            rx.clone(),
                            config.sample_rate,
                            0.0,
                            window_size,
                            file_path,
                        ),
                        _ => Err(anyhow::anyhow!("Unsupported sample format")),
                    };

                    match stream_res {
                        Ok((stream, stop_token, stream_recycle_tx, keep_alive)) => {
                            let max_frequency =
                                { shared_state_closure.lock().unwrap().max_frequency };
                            spawn_dsp_thread_with_recycle(
                                rx,
                                shared_state_closure.clone(),
                                config.sample_rate,
                                max_frequency,
                                window_size,
                                Some(stream_recycle_tx),
                            );
                            if dev_idx > 0
                                && !is_default_audio_device_selection(
                                    selected_device_name.as_deref(),
                                )
                                && let Ok(mut state) = shared_state_closure.lock()
                            {
                                eprintln!(
                                    "[audio] Switched to available fallback audio device: '{}'",
                                    device_name
                                );
                                state.selected_audio_device = Some(device_name.clone());
                            }
                            return Ok(PlaybackHandle::Cpal(stream, stop_token, keep_alive));
                        }
                        Err(e) => {
                            last_cfg_err = e;
                        }
                    }
                }

                Err(last_cfg_err)
            })();

            match result {
                Ok(handle) => {
                    return Ok(handle);
                }
                Err(err) => {
                    if audio_source_opt.is_none() {
                        audio_source_opt = attempt_audio_source_opt;
                    }
                    eprintln!(
                        "Warning: Failed to create/play audio stream on device '{}': {:?}",
                        device_name, err
                    );
                    last_err = err;
                }
            }
        }

        Err(last_err)
    })();

    match cpal_stream_result {
        Ok(handle) => Ok(handle),
        Err(err) => {
            if mic {
                return Err(err);
            }
            eprintln!(
                "Warning: CPAL audio stream creation failed: {:?}. Falling back to silent dummy device.",
                err
            );

            if let Some(mut audio_source) = audio_source_opt
                .take()
                .or_else(|| load_audio_source(file_path).ok())
            {
                let sample_rate = target_rate;
                let window_size = calculate_power_of_two_window_size(sample_rate);
                let (tx, rx) = bounded::<DspMessage>(32);

                {
                    let mut state = shared_state.lock().unwrap();
                    state.is_dummy_audio_output = true;
                    populate_state_from_audio_source(
                        &mut state,
                        audio_source.as_mut(),
                        2,
                        sample_rate,
                    );
                }

                let max_frequency = { shared_state.lock().unwrap().max_frequency };
                spawn_dsp_thread(
                    rx,
                    shared_state.clone(),
                    sample_rate,
                    max_frequency,
                    window_size,
                );

                run_dummy(
                    audio_source,
                    shared_state,
                    tx,
                    sample_rate,
                    window_size,
                    file_path,
                )
            } else {
                Err(anyhow::anyhow!(
                    "CPAL audio stream creation failed: {:?}, and no audio source was available for fallback.",
                    err
                ))
            }
        }
    }
}

pub(in crate::audio) fn populate_state_from_audio_source(
    state: &mut AppState,
    audio_source: &mut dyn AudioSource,
    hardware_channels: i32,
    sample_rate: u32,
) {
    state.current_seconds = 0.0;
    state.lookahead_queue.clear();
    state.lookahead_sample_buffer.clear();
    state.lookahead_buffer_start_time = 0.0;
    state.lookahead_timeline = vec![0.0; 600 * 8];
    state.waveform_history.clear();
    state.raw_waveform.fill(0.0);
    state.track_ended = false;
    state.artist = audio_source.get_artist();
    state.module_type = audio_source.get_type();
    state.duration_seconds = audio_source.get_duration_seconds();
    let tracker_channels = audio_source.get_tracker_channels();
    state.hardware_channels = hardware_channels;
    let source_channels = audio_source.get_num_channels();
    state.source_channels = source_channels;
    state.is_native_atmos = audio_source.is_native_atmos();
    if hardware_channels <= 2
        || state.is_native_atmos
        || !crate::state::SurroundUpmixConfig::can_upmix(source_channels, hardware_channels)
    {
        state.surround_upmix.enabled = false;
    } else if !state.surround_upmix.user_overridden {
        state.surround_upmix.enabled = !state.force_stereo_downmix
            && !state.passthrough_enabled
            && crate::state::SurroundUpmixConfig::should_enable_by_default(
                source_channels,
                hardware_channels,
            );
    }
    state.num_channels = if state
        .surround_upmix
        .is_active_for(source_channels, hardware_channels)
    {
        hardware_channels.min(12)
    } else if let Some(tc) = tracker_channels {
        tc + 2
    } else {
        audio_source.get_num_channels()
    };
    state.channel_vus = vec![0.0; state.num_channels as usize];
    state.peak_vus = vec![0.0; state.num_channels as usize];
    state.bpm = audio_source.get_tempo();
    state.speed = audio_source.get_speed();
    state.video_info = audio_source.get_video_info();
    state.max_frequency = audio_source
        .get_intrinsic_sample_rate()
        .map(|r| r as f32 / 2.0)
        .unwrap_or(10000.0);
    state.current_sample_rate = sample_rate as f32;
    state.num_samples = audio_source.get_num_samples();
    state.num_instruments = audio_source.get_num_instruments();
    state.num_patterns = audio_source.get_num_patterns();
    state.bitrate = audio_source.get_bitrate();
    state.stats.audio_bitrate_kbps = state.bitrate;
    state.stats.video_fps = audio_source.get_video_fps();
    state.stats.video_bitrate_kbps = audio_source.get_video_bitrate();
    state.audio_tracks = audio_source.get_audio_tracks();
    state.selected_audio_track = audio_source.get_selected_audio_track();
    state.active_audio_tracks = audio_source.get_active_audio_tracks();
    state.audio_track_volumes = vec![1.0; state.audio_tracks.len()];
    state.multi_track_mix_mode = state.active_audio_tracks.len() > 1;
    state.audio_track_request = None;
    state.audio_mix_request = None;
    state.has_video_stream = audio_source.has_video_stream();
    state.tracker_channels = tracker_channels;
    state.tracker_patterns_by_order = std::sync::Arc::new(audio_source.pre_format_tracker_data());
    state.sync_rt_control();
}

pub(in crate::audio) fn push_chunk_lookahead_slices(
    state: &mut AppState,
    chunk: &AudioChunk,
    frames_read: usize,
    hardware_channels: usize,
    sample_rate: u32,
) {
    if frames_read == 0 || hardware_channels == 0 || sample_rate == 0 {
        return;
    }
    let slice_frames = ((sample_rate as f64 * 0.01).round() as usize).max(16);
    let valid_samples = frames_read * hardware_channels;

    let existing_end_time = state.lookahead_buffer_start_time
        + (state.lookahead_sample_buffer.len() / hardware_channels) as f64 / sample_rate as f64;
    if state.lookahead_sample_buffer.is_empty()
        || (chunk.current_seconds - existing_end_time).abs() > 0.05
    {
        state.lookahead_sample_buffer.clear();
        state.lookahead_buffer_start_time = chunk.current_seconds;
    }
    state
        .lookahead_sample_buffer
        .extend_from_slice(&chunk.samples[..valid_samples]);

    let buffer_frames = state.lookahead_sample_buffer.len() / hardware_channels;
    let full_slices = buffer_frames / slice_frames;

    for s in 0..full_slices {
        let start_f = s * slice_frames;
        let end_f = start_f + slice_frames;
        let slice_time = state.lookahead_buffer_start_time + (start_f as f64 / sample_rate as f64);

        let mut min_l = 0.0f32;
        let mut max_l = 0.0f32;
        let mut min_r = 0.0f32;
        let mut max_r = 0.0f32;
        let mut sq_l = 0.0f32;
        let mut sq_r = 0.0f32;
        let mut bass_acc = 0.0f32;
        let mut treb_acc = 0.0f32;

        let cnt = slice_frames as f32;
        let mut prev_l = 0.0f32;
        for i in start_f..end_f {
            let frame_slice =
                &state.lookahead_sample_buffer[i * hardware_channels..(i + 1) * hardware_channels];
            let l = get_downmixed_sample(hardware_channels as u16, frame_slice, 2, 0);
            let r = get_downmixed_sample(hardware_channels as u16, frame_slice, 2, 1);

            min_l = min_l.min(l);
            max_l = max_l.max(l);
            min_r = min_r.min(r);
            max_r = max_r.max(r);
            sq_l += l * l;
            sq_r += r * r;

            let mono = (l + r) * 0.5;
            let diff = if i > start_f { (l - prev_l).abs() } else { 0.0 };
            prev_l = l;

            bass_acc += mono.abs();
            treb_acc += diff * 2.0;
        }

        let rms_l = (sq_l / cnt).sqrt().min(1.0);
        let rms_r = (sq_r / cnt).sqrt().min(1.0);
        let bass = (bass_acc / cnt * 2.0).min(1.0);
        let treble = (treb_acc / cnt * 2.0).min(1.0);

        while let Some((t, _)) = state.lookahead_queue.back() {
            if *t >= slice_time - 0.0001 {
                state.lookahead_queue.pop_back();
            } else {
                break;
            }
        }

        state.lookahead_queue.push_back((
            slice_time,
            [
                min_l.clamp(-1.0, 1.0),
                max_l.clamp(-1.0, 1.0),
                min_r.clamp(-1.0, 1.0),
                max_r.clamp(-1.0, 1.0),
                rms_l,
                rms_r,
                bass,
                treble,
            ],
        ));
    }

    let consumed_samples = full_slices * slice_frames * hardware_channels;
    if consumed_samples > 0 {
        state.lookahead_sample_buffer.drain(0..consumed_samples);
        state.lookahead_buffer_start_time +=
            (full_slices * slice_frames) as f64 / sample_rate as f64;
    }

    while state.lookahead_queue.len() > 1800 {
        state.lookahead_queue.pop_front();
    }
    let AppState {
        ref lookahead_queue,
        ref raw_waveform,
        ref raw_channel_vus,
        current_seconds,
        ref mut lookahead_timeline,
        ..
    } = *state;
    update_lookahead_timeline_from_queue(
        lookahead_queue,
        raw_waveform,
        raw_channel_vus,
        current_seconds,
        lookahead_timeline,
    );
}

pub(in crate::audio) fn seed_audio_chunk_pool(
    free_tx: &mut rtrb::Producer<AudioChunk>,
    pool_size: usize,
    chunk_frames: usize,
    hardware_channels: usize,
) {
    for _ in 0..pool_size {
        let _ = free_tx.push(AudioChunk {
            samples: vec![0.0; chunk_frames * hardware_channels],
            valid_frames: 0,
            channel_vus: Vec::with_capacity(32),
            left_peak: 0.0,
            right_peak: 0.0,
            current_order: 0,
            current_row: 0,
            bpm: 0,
            speed: 0,
            current_seconds: 0.0,
            current_row_string: String::with_capacity(128),
            tracker_channels: None,
            spatial_channels: 0,
            track_ended: false,
            seek_epoch: 0,
        });
    }
}

#[inline]
pub(in crate::audio) fn flush_stale_ready_chunks(
    ready_rx: &mut rtrb::Consumer<AudioChunk>,
    free_tx: &mut rtrb::Producer<AudioChunk>,
    latest_seek_epoch: u64,
) {
    while let Ok(peeked) = ready_rx.peek() {
        if peeked.seek_epoch < latest_seek_epoch {
            if let Ok(stale) = ready_rx.pop() {
                let _ = free_tx.push(stale);
            }
        } else {
            break;
        }
    }
}

fn pop_ready_with_timeout(
    ready_rx: &mut rtrb::Consumer<AudioChunk>,
    free_tx: &mut rtrb::Producer<AudioChunk>,
    stop_token: &std::sync::atomic::AtomicBool,
    latest_seek_epoch: u64,
    timeout: std::time::Duration,
) -> Option<AudioChunk> {
    let deadline = Instant::now() + timeout;
    loop {
        flush_stale_ready_chunks(ready_rx, free_tx, latest_seek_epoch);
        if let Ok(chunk) = ready_rx.pop() {
            return Some(chunk);
        }
        if stop_token.load(std::sync::atomic::Ordering::Relaxed)
            || ready_rx.is_abandoned()
            || Instant::now() >= deadline
        {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

pub(in crate::audio) fn process_decoder_state_requests(
    state: &mut AppState,
    audio_source: &mut dyn AudioSource,
    mut clear_video_packets: impl FnMut(),
) -> bool {
    let mut seeked = false;
    if let Some(mix_req) = state.audio_mix_request.take()
        && !mix_req.is_empty()
    {
        let mut new_active: Vec<usize> = mix_req.iter().map(|&(idx, _)| idx).collect();
        new_active.sort();
        let current_active = audio_source.get_active_audio_tracks();
        let tracks_changed = new_active != current_active;
        let play_pos = state.current_seconds;
        if audio_source.set_active_audio_tracks(&mix_req).is_ok() {
            if tracks_changed {
                audio_source.set_position_seconds(play_pos);
                state.audio_track_request = None;
            }
            state.active_audio_tracks = audio_source.get_active_audio_tracks();
            if let Some(&first_idx) = state.active_audio_tracks.first() {
                state.selected_audio_track = first_idx;
            }
            let source_channels = audio_source.get_num_channels();
            state.source_channels = source_channels;
            state.is_native_atmos = audio_source.is_native_atmos();
            if state.hardware_channels <= 2
                || state.is_native_atmos
                || !crate::state::SurroundUpmixConfig::can_upmix(
                    source_channels,
                    state.hardware_channels,
                )
            {
                state.surround_upmix.enabled = false;
            } else if !state.surround_upmix.user_overridden {
                state.surround_upmix.enabled = !state.force_stereo_downmix
                    && !state.passthrough_enabled
                    && crate::state::SurroundUpmixConfig::should_enable_by_default(
                        source_channels,
                        state.hardware_channels,
                    );
            }
            state.num_channels = if state
                .surround_upmix
                .is_active_for(source_channels, state.hardware_channels)
            {
                state.hardware_channels.min(12)
            } else {
                audio_source.get_num_channels()
            };
            let ch_count = state.num_channels as usize;
            state.channel_vus.resize(ch_count, 0.0);
            state.peak_vus.resize(ch_count, 0.0);
            state.module_type = audio_source.get_type();
            state.bitrate = audio_source.get_bitrate();
            state.stats.audio_bitrate_kbps = state.bitrate;
            state.multi_track_mix_mode = state.active_audio_tracks.len() > 1;

            let num_tracks = state.audio_tracks.len().max(1);
            if state.audio_track_volumes.len() != num_tracks {
                state.audio_track_volumes = vec![1.0; num_tracks];
            }
            for &(idx, vol) in &mix_req {
                if idx < state.audio_track_volumes.len() {
                    state.audio_track_volumes[idx] = vol;
                }
            }

            if tracks_changed {
                let mix_desc = if state.active_audio_tracks.len() > 1 {
                    let track_nums: Vec<String> = state
                        .active_audio_tracks
                        .iter()
                        .map(|idx| (idx + 1).to_string())
                        .collect();
                    format!("🎛 Audio Mix: Tracks {}", track_nums.join("+"))
                } else {
                    let idx = state.selected_audio_track;
                    let track_title = state
                        .audio_tracks
                        .get(idx)
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| format!("Track {}", idx + 1));
                    if track_title.to_lowercase().starts_with("track") {
                        format!("🎛 {}", track_title)
                    } else {
                        format!("🎛 Track {}: {}", idx + 1, track_title)
                    }
                };
                state.osd_text = Some(mix_desc);
                state.osd_timer = 3.0;

                state.lookahead_queue.clear();
                state.lookahead_sample_buffer.clear();
                state.lookahead_timeline.fill(0.0);
                state.lookahead_buffer_start_time = play_pos;
                state.waveform_history.clear();
                state.seek_epoch += 1;
                seeked = true;
            }
        }
    }
    if let Some(track_idx) = state.audio_track_request.take()
        && state.audio_tracks.len() > 1
        && track_idx < state.audio_tracks.len()
        && (audio_source.get_active_audio_tracks() != vec![track_idx])
    {
        let play_pos = state.current_seconds;
        if audio_source.select_audio_track(track_idx).is_ok() {
            audio_source.set_position_seconds(play_pos);
            state.selected_audio_track = track_idx;
            state.active_audio_tracks = vec![track_idx];
            state.multi_track_mix_mode = false;
            let source_channels = audio_source.get_num_channels();
            state.source_channels = source_channels;
            state.is_native_atmos = audio_source.is_native_atmos();
            if state.hardware_channels <= 2
                || state.is_native_atmos
                || !crate::state::SurroundUpmixConfig::can_upmix(
                    source_channels,
                    state.hardware_channels,
                )
            {
                state.surround_upmix.enabled = false;
            } else if !state.surround_upmix.user_overridden {
                state.surround_upmix.enabled = !state.force_stereo_downmix
                    && !state.passthrough_enabled
                    && crate::state::SurroundUpmixConfig::should_enable_by_default(
                        source_channels,
                        state.hardware_channels,
                    );
            }
            state.num_channels = if state
                .surround_upmix
                .is_active_for(source_channels, state.hardware_channels)
            {
                state.hardware_channels.min(12)
            } else {
                audio_source.get_num_channels()
            };
            let ch_count = state.num_channels as usize;
            state.channel_vus.resize(ch_count, 0.0);
            state.peak_vus.resize(ch_count, 0.0);
            state.module_type = audio_source.get_type();
            state.bitrate = audio_source.get_bitrate();
            state.stats.audio_bitrate_kbps = state.bitrate;
            let track_title = state
                .audio_tracks
                .get(track_idx)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| format!("Track {}", track_idx + 1));
            let display_title = if track_title.to_lowercase().starts_with("track") {
                format!("🎛 {}", track_title)
            } else {
                format!("🎛 Track {}: {}", track_idx + 1, track_title)
            };
            state.osd_text = Some(display_title);
            state.osd_timer = 3.0;

            state.lookahead_queue.clear();
            state.lookahead_sample_buffer.clear();
            state.lookahead_timeline.fill(0.0);
            state.lookahead_buffer_start_time = play_pos;
            state.waveform_history.clear();
            state.seek_epoch += 1;
            seeked = true;
        }
    }
    if let Some(pos) = state.seek_request.take() {
        audio_source.set_position_seconds(pos);
        state.current_seconds = pos;
        state.track_ended = false;
        state.seek_epoch += 1;
        state.lookahead_queue.clear();
        state.lookahead_sample_buffer.clear();
        state.lookahead_timeline.fill(0.0);
        state.lookahead_buffer_start_time = pos;
        state.waveform_history.clear();
        clear_video_packets();
        seeked = true;
    }
    if seeked {
        state.sync_rt_control();
    }
    seeked
}

pub(in crate::audio) fn decode_single_audio_chunk(
    audio_source: &mut dyn AudioSource,
    chunk: &mut AudioChunk,
    hardware_channels: usize,
    sample_rate: u32,
    chunk_frames: usize,
) -> (usize, f32, u32) {
    let chunk_start_seconds = audio_source.get_position_seconds();
    let decode_start = Instant::now();
    let frames_read = audio_source.read_frames(
        hardware_channels,
        sample_rate,
        &mut chunk.samples[..chunk_frames * hardware_channels],
    );
    let decode_elapsed = decode_start.elapsed().as_micros() as f32;

    chunk.valid_frames = frames_read;
    chunk.current_order = audio_source.get_current_order();
    chunk.current_row = audio_source.get_current_row();
    chunk.bpm = audio_source.get_tempo();
    chunk.speed = audio_source.get_speed();
    chunk.current_seconds = if audio_source.has_video_stream() && frames_read > 0 {
        (audio_source.get_position_seconds() - (frames_read as f64 / sample_rate.max(1) as f64))
            .max(0.0)
    } else {
        chunk_start_seconds
    };
    chunk.current_row_string.clear();
    chunk
        .current_row_string
        .push_str(&audio_source.get_current_row_string());
    chunk.tracker_channels = audio_source.get_tracker_channels();
    chunk.spatial_channels = audio_source.get_num_channels();
    chunk.track_ended = frames_read == 0;

    let mut left_peak = 0.0_f32;
    let mut right_peak = 0.0_f32;
    let mut clips = 0u32;
    for i in 0..frames_read {
        let l_val = chunk.samples[i * hardware_channels];
        left_peak = left_peak.max(l_val.abs());
        if l_val.abs() > 1.0001 {
            clips += 1;
        }

        if hardware_channels > 1 {
            let r_val = chunk.samples[i * hardware_channels + 1];
            right_peak = right_peak.max(r_val.abs());
            if r_val.abs() > 1.0001 {
                clips += 1;
            }
        } else {
            right_peak = left_peak;
        }
    }
    chunk.left_peak = left_peak;
    chunk.right_peak = right_peak;

    chunk.channel_vus.clear();
    if let Some(num_mod_channels) = chunk.tracker_channels {
        chunk.channel_vus.push(left_peak);
        for i in 0..num_mod_channels {
            chunk
                .channel_vus
                .push(audio_source.get_current_channel_vu_mono(i));
        }
        chunk.channel_vus.push(right_peak);
    } else {
        for i in 0..chunk.spatial_channels {
            chunk
                .channel_vus
                .push(audio_source.get_current_channel_vu_mono(i));
        }
    }

    (frames_read, decode_elapsed, clips)
}

#[inline]
pub(in crate::audio) fn compute_fft_mono_sample(frame_slice: &[f32], src_ch: usize) -> f32 {
    let ch_count = src_ch.min(frame_slice.len()).max(1);
    let mut mono = 0.0f32;
    for (c, &s) in frame_slice.iter().enumerate().take(ch_count) {
        let w = match c {
            0 | 1 => {
                if ch_count == 1 {
                    1.0
                } else {
                    0.5
                }
            }
            2 => std::f32::consts::FRAC_1_SQRT_2,
            _ => 0.25,
        };
        mono += s * w;
    }
    mono.clamp(-1.0, 1.0)
}

fn run_dummy(
    audio_source: Box<dyn AudioSource>,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    sample_rate: u32,
    window_size: usize,
    current_path: &str,
) -> Result<PlaybackHandle> {
    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_token_clone = stop_token.clone();
    let hardware_channels = 2;
    let chunk_frames = 1024;
    let pool_size = ((sample_rate as usize * 8) / chunk_frames).clamp(384, 1024);

    let (ready_tx, mut ready_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size);
    let (mut free_tx, free_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size + 16);
    seed_audio_chunk_pool(&mut free_tx, pool_size, chunk_frames, hardware_channels);
    let (shutdown_tx, shutdown_rx) = bounded::<()>(1);

    spawn_media_decoder_threads(
        audio_source,
        shared_state.clone(),
        stop_token.clone(),
        ready_tx,
        free_rx,
        hardware_channels,
        sample_rate,
        chunk_frames,
        pool_size,
        0,
        current_path,
        Some(shutdown_rx),
    );

    let shared_state_dummy = shared_state.clone();

    let thread_handle = std::thread::spawn(move || {
        let _shutdown_guard = shutdown_tx;
        let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
        let mut channel_fft_buffers: Vec<Vec<f32>> =
            vec![vec![0.0; window_size]; hardware_channels.max(12)];
        let mut fft_index = 0;
        let recycle_rx = dsp_recycle_channel().1.clone();

        let (mut was_paused, mut latest_seek_epoch, mut upmix_cfg) = {
            let state = shared_state_dummy.lock().unwrap();
            (state.is_paused, state.seek_epoch, state.surround_upmix)
        };
        let mut upmixer_dsp = SurroundUpmixerDsp::new(sample_rate);
        let mut upmix_peak_vus = [0.0f32; 12];

        while !stop_token_clone.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(state) = shared_state_dummy.try_lock() {
                was_paused = state.is_paused;
                latest_seek_epoch = state.seek_epoch;
                upmix_cfg = state.surround_upmix;
            }
            flush_stale_ready_chunks(&mut ready_rx, &mut free_tx, latest_seek_epoch);

            if was_paused {
                if let Ok(mut state) = shared_state_dummy.try_lock() {
                    state.raw_channel_vus.fill(0.0);
                    state.raw_spectrum_data.fill(0.0);
                    state.gpu_spectrum_data.fill(0.0);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }

            match pop_ready_with_timeout(
                &mut ready_rx,
                &mut free_tx,
                &stop_token_clone,
                latest_seek_epoch,
                std::time::Duration::from_millis(50),
            ) {
                Some(chunk) => {
                    if chunk.seek_epoch < latest_seek_epoch {
                        let _ = free_tx.push(chunk);
                        continue;
                    }
                    if chunk.track_ended
                        && let Ok(mut state) = shared_state_dummy.try_lock()
                    {
                        state.track_ended = true;
                    }

                    // Process chunk for FFT and VU levels (dummy device is 2ch stereo, so surround upmix is bypassed)
                    let to_copy = chunk.valid_frames;
                    let upmix_active =
                        upmix_cfg.is_active_for(chunk.spatial_channels, hardware_channels as i32);
                    if upmix_active {
                        upmixer_dsp.sync_config(&upmix_cfg, sample_rate);
                        upmix_peak_vus = [0.0f32; 12];
                        let src_ch = (chunk.spatial_channels as usize).max(1);
                        let active_ch = hardware_channels.min(12);
                        for i in 0..to_copy {
                            let frame_slice =
                                &chunk.samples[i * hardware_channels..(i + 1) * hardware_channels];
                            let out_channels = upmixer_dsp.process_multichannel_frame_for_hardware(
                                frame_slice,
                                src_ch,
                                &upmix_cfg,
                                hardware_channels,
                            );
                            for c in 0..active_ch {
                                let s = out_channels[c];
                                upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                channel_fft_buffers[c][fft_index] = s;
                            }
                            let mono = compute_fft_mono_sample(frame_slice, src_ch);
                            fft_buffer[fft_index] = mono;
                            fft_index = (fft_index + 1) % window_size;
                        }
                    } else {
                        for i in 0..to_copy {
                            let mut mono = 0.0;
                            for (c, buf) in channel_fft_buffers
                                .iter_mut()
                                .enumerate()
                                .take(hardware_channels)
                            {
                                let sample =
                                    chunk.samples[i * hardware_channels + c].clamp(-1.0, 1.0);
                                let ch_weight = match c {
                                    0 | 1 => {
                                        if hardware_channels == 1 {
                                            1.0
                                        } else {
                                            0.5
                                        }
                                    }
                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                    _ => 0.25,
                                };
                                mono += sample * ch_weight;
                                buf[fft_index] = sample;
                            }
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    }

                    let (dsp_channels, vu_slice): (usize, &[f32]) = if upmix_active {
                        let active_ch = hardware_channels.min(12);
                        (active_ch, &upmix_peak_vus[..active_ch])
                    } else {
                        (
                            hardware_channels.min(chunk.channel_vus.len().max(1)),
                            &chunk.channel_vus,
                        )
                    };

                    let msg = build_recycled_dsp_message(
                        &recycle_rx,
                        &fft_buffer,
                        &channel_fft_buffers,
                        fft_index,
                        window_size,
                        dsp_channels,
                        vu_slice,
                        chunk.current_order,
                        chunk.current_row,
                        chunk.bpm,
                        chunk.speed,
                        chunk.current_seconds,
                        &chunk.current_row_string,
                        chunk.seek_epoch,
                        true,
                    );

                    let _ = tx.try_send(msg);

                    if let Ok(mut state) = shared_state_dummy.try_lock()
                        && chunk.seek_epoch == state.seek_epoch
                    {
                        state.current_seconds = chunk.current_seconds;
                    }

                    // Sleep to simulate the playback duration of this chunk
                    let chunk_duration_ms = (to_copy as f64 / sample_rate as f64 * 1000.0) as u64;
                    if chunk_duration_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(chunk_duration_ms));
                    }

                    let _ = free_tx.push(chunk);
                }
                None => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    });

    Ok(PlaybackHandle::Dummy(Some(thread_handle), stop_token))
}

#[cfg(target_os = "linux")]
fn run_linux_pipewire_mpegh_714(
    audio_source_opt: &mut Option<Box<dyn AudioSource>>,
    shared_state: Arc<Mutex<AppState>>,
    file_path: &str,
) -> Result<PlaybackHandle> {
    use std::io::Write;

    let mut mpegh_encoder = crate::mpegh::Mpegh714Encoder::new(4, true)?;
    let mut child = std::process::Command::new("pw-play")
        .arg("--properties=node.passthrough=true")
        .arg("-f")
        .arg("s16")
        .arg("-r")
        .arg("192000")
        .arg("-c")
        .arg("2")
        .arg("--raw")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn pw-play for MPEG-H 3D Audio 7.1.4 passthrough")?;

    std::thread::sleep(std::time::Duration::from_millis(80));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(anyhow::anyhow!(
            "pw-play exited immediately with status: {}",
            status
        ));
    }

    let mut pw_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open pw-play stdin"))?;

    let mut audio_source = audio_source_opt
        .take()
        .ok_or_else(|| anyhow::anyhow!("No audio source available"))?;

    let sample_rate = crate::mpegh::MPEGH_SAMPLE_RATE;
    let hardware_channels = 12usize;
    let max_frequency = {
        let mut state = shared_state.lock().unwrap();
        populate_state_from_audio_source(
            &mut state,
            audio_source.as_mut(),
            hardware_channels as i32,
            sample_rate,
        );
        state.stats.mpegh_bitstream_active = true;
        state.stats.mpegh_carrier_info = "PipeWire IEC 61937-13 MPEG-H (2ch @ 192 kHz)".to_string();
        state.max_frequency
    };

    let (tx, rx) = bounded::<DspMessage>(32);
    let window_size = calculate_power_of_two_window_size(sample_rate);
    spawn_dsp_thread(
        rx,
        shared_state.clone(),
        sample_rate,
        max_frequency,
        window_size,
    );

    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let chunk_frames = 1024;
    let pool_size = ((sample_rate as usize * 8) / chunk_frames).clamp(384, 1024);
    let (ready_tx, mut ready_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size);
    let (mut free_tx, free_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size + 16);
    seed_audio_chunk_pool(&mut free_tx, pool_size, chunk_frames, hardware_channels);
    let (shutdown_tx, shutdown_rx) = bounded::<()>(1);

    spawn_media_decoder_threads(
        audio_source,
        shared_state.clone(),
        stop_token.clone(),
        ready_tx,
        free_rx,
        hardware_channels,
        sample_rate,
        chunk_frames,
        pool_size,
        4,
        file_path,
        Some(shutdown_rx),
    );

    let stop_token_thread = stop_token.clone();
    let shared_state_thread = shared_state.clone();
    let thread_handle = std::thread::spawn(move || {
        let _shutdown_guard = shutdown_tx;
        let mut fft_buffer = vec![0.0f32; window_size];
        let mut channel_fft_buffers = vec![vec![0.0f32; window_size]; 12];
        let mut fft_index = 0usize;
        let recycle_rx = dsp_recycle_channel().1.clone();
        let mut upmixer_dsp = SurroundUpmixerDsp::new(sample_rate);
        let mut upmix_peak_vus = [0.0f32; 12];
        let (mut latest_seek_epoch, rt_control_thread) = {
            let mut s = shared_state_thread.lock().unwrap();
            s.sync_rt_control();
            (s.seek_epoch, Arc::clone(&s.rt_control))
        };
        let mut iec_byte_queue: std::collections::VecDeque<u8> =
            std::collections::VecDeque::with_capacity(65536);
        let mut fatal_pipe_err = false;

        while !stop_token_thread.load(std::sync::atomic::Ordering::Relaxed) {
            let is_paused = rt_control_thread.is_paused();
            let ctrl_epoch = rt_control_thread.seek_epoch();
            if ctrl_epoch > latest_seek_epoch {
                latest_seek_epoch = ctrl_epoch;
                iec_byte_queue.clear();
                mpegh_encoder.clear_accumulator();
            }
            flush_stale_ready_chunks(&mut ready_rx, &mut free_tx, latest_seek_epoch);
            let upmix_cfg = rt_control_thread.load_upmix_config();

            if is_paused {
                fft_buffer.fill(0.0);
                for ch_buf in &mut channel_fft_buffers {
                    ch_buf.fill(0.0);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }

            match pop_ready_with_timeout(
                &mut ready_rx,
                &mut free_tx,
                &stop_token_thread,
                latest_seek_epoch,
                std::time::Duration::from_millis(25),
            ) {
                Some(chunk) => {
                    if chunk.seek_epoch < latest_seek_epoch {
                        let _ = free_tx.push(chunk);
                        continue;
                    }
                    if chunk.seek_epoch > latest_seek_epoch {
                        latest_seek_epoch = chunk.seek_epoch;
                    }
                    if chunk.track_ended {
                        rt_control_thread.signal_track_ended();
                    }

                    let upmix_active = upmix_cfg.is_active_for_hardware(12);
                    if upmix_active {
                        upmixer_dsp.sync_config(&upmix_cfg, sample_rate);
                    }
                    let src_ch = (chunk.spatial_channels as usize).max(1);
                    let target_upmix = upmix_active && src_ch < 12;
                    let use_upmixer =
                        upmixer_dsp.is_blending_or_active(target_upmix) && src_ch < 12;

                    for i in 0..chunk.valid_frames {
                        let frame_slice = &chunk.samples[i * 12..(i + 1) * 12];
                        if use_upmixer {
                            let out_channels = upmixer_dsp
                                .process_multichannel_frame_for_hardware_blended(
                                    frame_slice,
                                    src_ch,
                                    &upmix_cfg,
                                    12,
                                    target_upmix,
                                );
                            for (c, &raw_s) in out_channels.iter().enumerate() {
                                let s = raw_s.clamp(-1.0, 1.0);
                                upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                channel_fft_buffers[c][fft_index] = s;
                            }
                            let mono = compute_fft_mono_sample(frame_slice, src_ch);
                            fft_buffer[fft_index] = mono;
                            fft_index = (fft_index + 1) % window_size;

                            if let Ok(true) = mpegh_encoder
                                .push_rusttracker_714_frame(&out_channels, &mut iec_byte_queue)
                            {
                                let (s1, s2) = iec_byte_queue.as_slices();
                                if pw_stdin.write_all(s1).is_err()
                                    || pw_stdin.write_all(s2).is_err()
                                {
                                    fatal_pipe_err = true;
                                    break;
                                }
                                iec_byte_queue.clear();
                            }
                        } else {
                            let mut rt_frame = [0.0f32; 12];
                            for (dst, &src) in rt_frame.iter_mut().zip(&frame_slice[..12]) {
                                *dst = src.clamp(-1.0, 1.0);
                            }
                            if src_ch >= 8 {
                                rt_frame.swap(4, 6);
                                rt_frame.swap(5, 7);
                            }
                            let mut mono = 0.0f32;
                            for (c, &s) in rt_frame.iter().enumerate() {
                                let ch_weight = match c {
                                    0 | 1 => 0.5,
                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                    _ => 0.25,
                                };
                                mono += s * ch_weight;
                                channel_fft_buffers[c][fft_index] = s;
                            }
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;

                            if let Ok(true) = mpegh_encoder
                                .push_rusttracker_714_frame(&rt_frame, &mut iec_byte_queue)
                            {
                                let (s1, s2) = iec_byte_queue.as_slices();
                                if pw_stdin.write_all(s1).is_err()
                                    || pw_stdin.write_all(s2).is_err()
                                {
                                    fatal_pipe_err = true;
                                    break;
                                }
                                iec_byte_queue.clear();
                            }
                        }
                    }

                    if fatal_pipe_err {
                        let _ = free_tx.push(chunk);
                        rt_control_thread.signal_audio_device_lost();
                        break;
                    }

                    let (dsp_channels, vu_slice): (usize, &[f32]) = if upmix_active && src_ch < 12 {
                        (12, &upmix_peak_vus)
                    } else {
                        (12.min(chunk.channel_vus.len().max(1)), &chunk.channel_vus)
                    };
                    let msg = build_recycled_dsp_message(
                        &recycle_rx,
                        &fft_buffer,
                        &channel_fft_buffers,
                        fft_index,
                        window_size,
                        dsp_channels,
                        vu_slice,
                        chunk.current_order,
                        chunk.current_row,
                        chunk.bpm,
                        chunk.speed,
                        chunk.current_seconds,
                        &chunk.current_row_string,
                        chunk.seek_epoch,
                        true,
                    );
                    upmix_peak_vus = [0.0f32; 12];
                    let _ = tx.try_send(msg);
                    let _ = free_tx.push(chunk);
                }
                None => {
                    if ready_rx.is_abandoned() {
                        break;
                    }
                    continue;
                }
            }
        }

        drop(pw_stdin);
        let _ = child.kill();
        let _ = child.wait();
    });

    Ok(PlaybackHandle::Dummy(Some(thread_handle), stop_token))
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_wasapi_exclusive_multichannel(
    selected_device_name: Option<&str>,
    audio_source_opt: &mut Option<Box<dyn AudioSource>>,
    shared_state: Arc<Mutex<AppState>>,
    target_rate: u32,
    _target_channels: u16,
    _surround_upmix_enabled: bool,
    mpegh_bitstream: bool,
    allow_exclusive: bool,
    file_path: &str,
) -> Result<PlaybackHandle> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        WaitForSingleObject,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::core::{GUID, Interface};

    struct ComGuard(bool);
    impl ComGuard {
        fn init_multithreaded() -> Self {
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            Self(hr.is_ok())
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }

    struct OwnedHandle(HANDLE);
    impl OwnedHandle {
        fn new(handle: HANDLE) -> Self {
            Self(handle)
        }
        fn as_raw(&self) -> HANDLE {
            self.0
        }
        fn into_raw(mut self) -> HANDLE {
            let h = self.0;
            self.0 = HANDLE::default();
            h
        }
        fn reset(&mut self, handle: HANDLE) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
            self.0 = handle;
        }
        fn clear(&mut self) {
            self.reset(HANDLE::default());
        }
    }
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            self.clear();
        }
    }

    struct CoTaskMemPwstr(windows::core::PWSTR);
    impl CoTaskMemPwstr {
        fn to_string_lossy(&self) -> Option<String> {
            if self.0.is_null() {
                None
            } else {
                unsafe { self.0.to_string().ok() }
            }
        }
    }
    impl Drop for CoTaskMemPwstr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CoTaskMemFree(Some(self.0.0 as *const std::ffi::c_void));
                }
            }
        }
    }

    struct CoTaskMemWaveFormat(*mut WAVEFORMATEX);
    impl Drop for CoTaskMemWaveFormat {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CoTaskMemFree(Some(self.0 as *const std::ffi::c_void));
                }
            }
        }
    }

    fn get_wasapi_device_id(device: &IMMDevice) -> String {
        unsafe {
            device
                .GetId()
                .ok()
                .map(CoTaskMemPwstr)
                .and_then(|s| s.to_string_lossy())
                .unwrap_or_default()
        }
    }

    /// Returns `Some(pid)` if another process currently has an active audio session
    /// (`AudioSessionStateActive`) on `device`.
    fn device_has_other_active_audio_session(device: &IMMDevice) -> Option<u32> {
        let self_pid = std::process::id();
        unsafe {
            let mgr: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
            let enumerator = mgr.GetSessionEnumerator().ok()?;
            let count = enumerator.GetCount().ok()?;
            for i in 0..count {
                let Ok(ctrl) = enumerator.GetSession(i) else {
                    continue;
                };
                let Ok(state) = ctrl.GetState() else {
                    continue;
                };
                if state != AudioSessionStateActive {
                    continue;
                }
                let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let Ok(pid) = ctrl2.GetProcessId() else {
                    continue;
                };
                if pid != 0 && pid != self_pid {
                    return Some(pid);
                }
            }
            None
        }
    }

    const WAIT_OBJECT_0: u32 = 0;
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
    const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    #[derive(Clone, Copy, Debug)]
    enum WasapiPcmFormat {
        I24In32,
        F32,
        I32,
        I16,
    }

    impl WasapiPcmFormat {
        fn container_bits(self) -> u16 {
            match self {
                Self::I24In32 | Self::F32 | Self::I32 => 32,
                Self::I16 => 16,
            }
        }
        fn valid_bits(self) -> u16 {
            match self {
                Self::I24In32 => 24,
                Self::F32 | Self::I32 => 32,
                Self::I16 => 16,
            }
        }
        fn sub_format(self) -> GUID {
            match self {
                Self::F32 => KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
                Self::I24In32 | Self::I32 | Self::I16 => KSDATAFORMAT_SUBTYPE_PCM,
            }
        }
        fn bytes_per_sample(self) -> usize {
            (self.container_bits() / 8) as usize
        }
        fn label(self) -> &'static str {
            match self {
                Self::I24In32 => "24-bit PCM (in 32-bit container)",
                Self::F32 => "32-bit IEEE Float",
                Self::I32 => "32-bit PCM",
                Self::I16 => "16-bit PCM",
            }
        }
    }

    fn build_wave_format(
        channels: u16,
        rate: u32,
        channel_mask: u32,
        fmt: WasapiPcmFormat,
    ) -> WAVEFORMATEXTENSIBLE {
        let container_bits = fmt.container_bits();
        let block_align = channels * (container_bits / 8);
        let avg_bytes = rate * block_align as u32;
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: 0xFFFE,
                nChannels: channels,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: avg_bytes,
                nBlockAlign: block_align,
                wBitsPerSample: container_bits,
                cbSize: 22,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: fmt.valid_bits(),
            },
            dwChannelMask: channel_mask,
            SubFormat: fmt.sub_format(),
        }
    }

    fn get_wasapi_device_name(device: &IMMDevice) -> Option<String> {
        unsafe {
            let store = device.OpenPropertyStore(STGM_READ).ok()?;
            let propvar = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
            if propvar.Anonymous.Anonymous.vt == VT_LPWSTR {
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

    fn try_open_exclusive_client(
        device: &IMMDevice,
        channels: u16,
        rate: u32,
        channel_mask: u32,
        fmt: WasapiPcmFormat,
    ) -> Result<(IAudioClient, IAudioRenderClient, HANDLE, u32)> {
        const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR: i32 = -2004287463i32; // 0x88890019

        let wf = build_wave_format(channels, rate, channel_mask, fmt);
        let wf_ptr = &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;

        let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        if unsafe { audio_client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr, None) }
            != windows::core::HRESULT(0)
        {
            return Err(anyhow::anyhow!(
                "Format unsupported: {}ch @ {} Hz ({})",
                channels,
                rate,
                fmt.label()
            ));
        }

        let mut default_period = 0i64;
        let mut min_period = 0i64;
        unsafe {
            let _ = audio_client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period));
        }
        let mut buffer_duration = if default_period > 0 {
            default_period.max(200_000)
        } else if min_period > 0 {
            min_period.max(200_000)
        } else {
            200_000
        };

        let init_res = unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                buffer_duration,
                buffer_duration,
                wf_ptr,
                None,
            )
        };

        let active_client = match init_res {
            Ok(()) => audio_client,
            Err(e) if e.code().0 == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR => {
                let aligned_frames = unsafe { audio_client.GetBufferSize()? };
                buffer_duration =
                    ((10_000_000.0 * aligned_frames as f64 / rate as f64) + 0.5) as i64;
                drop(audio_client);
                let retry_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
                unsafe {
                    retry_client
                        .Initialize(
                            AUDCLNT_SHAREMODE_EXCLUSIVE,
                            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                            buffer_duration,
                            buffer_duration,
                            wf_ptr,
                            None,
                        )
                        .map_err(|e| anyhow::anyhow!(e))?;
                }
                retry_client
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        };

        let event = OwnedHandle::new(unsafe { CreateEventW(None, false, false, None)? });
        unsafe {
            active_client
                .SetEventHandle(event.as_raw())
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        let buffer_frames = unsafe { active_client.GetBufferSize()? };
        let render_client: IAudioRenderClient =
            unsafe { active_client.GetService().map_err(|e| anyhow::anyhow!(e))? };
        Ok((
            active_client,
            render_client,
            event.into_raw(),
            buffer_frames,
        ))
    }

    /// Negotiate a WASAPI Exclusive Mode IEC 61937-13 bitstream carrier for 12-channel (`7.1.4` / `11.1`)
    /// MPEG-H 3D Audio (`CICP Layout 19`) to an HDMI AVR (such as the Denon AVR-X3800H).
    ///
    /// First probes dedicated MPEG-H IEC 61937 subtype GUIDs (`00000019-0cea-...` and `0000000f-0cea-...`).
    /// When the endpoint is a multichannel HDMI AVR (or the user explicitly enabled MPEG-H 11.1) and the
    /// Windows GPU HDMI driver only registers legacy CEA-861-D/E IEC 61937 subtypes in `IsFormatSupported`,
    /// also probes high-bitrate 192 kHz IEC 61937 carriers (`E-AC-3` 2ch @ 192 kHz = 6.144 Mbps `rate_factor = 4`,
    /// `DTS-HD` / `MLP` 8ch @ 192 kHz = 24.576 Mbps `rate_factor = 16`), because the AVR decodes the stream
    /// from the embedded IEC 61937-13 `Pa/Pb/Pc` burst preamble (`Pc = 0x0019`).
    fn try_open_exclusive_mpegh_iec61937_client(
        device: &IMMDevice,
        allow_cea_carrier_fallback: bool,
    ) -> Result<(IAudioClient, IAudioRenderClient, HANDLE, u32, usize, String)> {
        const AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR: i32 = -2004287463i32; // 0x88890019

        // CEA-861.3 / IEC 61937-13 MPEG-H 3D Audio and compatible 192 kHz HDMI IEC 61937 carriers:
        // (guid_data1, channels, rate_hz, channel_mask, rate_factor, carrier_label, requires_avr_fallback)
        let candidates: &[(u32, u16, u32, u32, usize, &str, bool)] = &[
            (
                0x00000019,
                2,
                192_000,
                0x3,
                4,
                "IEC 61937-13 MPEG-H (2ch @ 192 kHz)",
                false,
            ),
            (
                0x0000000f,
                2,
                192_000,
                0x3,
                4,
                "CEA-861-G MPEG-H 3D Audio (2ch @ 192 kHz)",
                false,
            ),
            (
                0x00000019,
                8,
                192_000,
                0x63F,
                16,
                "IEC 61937-13 MPEG-H HBR (8ch @ 192 kHz)",
                false,
            ),
            (
                0x0000000f,
                8,
                192_000,
                0x63F,
                16,
                "CEA-861-G MPEG-H 3D Audio HBR (8ch @ 192 kHz)",
                false,
            ),
            (
                0x0000000a,
                2,
                192_000,
                0x3,
                4,
                "IEC 61937-13 over HDMI 192 kHz 2ch carrier",
                true,
            ),
            (
                0x0000000b,
                8,
                192_000,
                0x63F,
                16,
                "IEC 61937-13 over HDMI 192 kHz 8ch HBR carrier",
                true,
            ),
            (
                0x0000000c,
                8,
                192_000,
                0x63F,
                16,
                "IEC 61937-13 over HDMI 192 kHz 8ch MAT/MLP carrier",
                true,
            ),
        ];

        #[repr(C, packed)]
        struct WaveFormatExtensibleIec61937 {
            format_ext: WAVEFORMATEXTENSIBLE,
            dw_encoded_samples_per_sec: u32,
            dw_encoded_channel_count: u32,
            dw_average_bytes_per_sec: u32,
        }

        let mut last_err = anyhow::anyhow!("No IEC 61937-13 MPEG-H carrier supported on endpoint");

        for &(
            guid_data1,
            carrier_ch,
            carrier_rate,
            carrier_mask,
            rate_factor,
            label,
            needs_fallback,
        ) in candidates
        {
            if needs_fallback && !allow_cea_carrier_fallback {
                continue;
            }
            let sub_format = GUID {
                data1: guid_data1,
                data2: 0x0cea,
                data3: 0x0010,
                data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
            };
            let block_align = carrier_ch * 2; // 16-bit IEC 61937 words
            let avg_bytes = carrier_rate * block_align as u32;

            // Try both WAVEFORMATEXTENSIBLE_IEC61937 (cbSize = 34, standard for >48 kHz HDMI IEC 61937)
            // and legacy WAVEFORMATEXTENSIBLE (cbSize = 22).
            for &use_iec_ext in &[true, false] {
                let wf_ext = WAVEFORMATEXTENSIBLE {
                    Format: WAVEFORMATEX {
                        wFormatTag: 0xFFFE,
                        nChannels: carrier_ch,
                        nSamplesPerSec: carrier_rate,
                        nAvgBytesPerSec: avg_bytes,
                        nBlockAlign: block_align,
                        wBitsPerSample: 16,
                        cbSize: if use_iec_ext { 34 } else { 22 },
                    },
                    Samples: WAVEFORMATEXTENSIBLE_0 {
                        wValidBitsPerSample: 16,
                    },
                    dwChannelMask: carrier_mask,
                    SubFormat: sub_format,
                };
                let wf_iec = WaveFormatExtensibleIec61937 {
                    format_ext: wf_ext,
                    dw_encoded_samples_per_sec: carrier_rate,
                    dw_encoded_channel_count: carrier_ch as u32,
                    dw_average_bytes_per_sec: avg_bytes,
                };
                let wf_ptr = if use_iec_ext {
                    &wf_iec as *const WaveFormatExtensibleIec61937 as *const WAVEFORMATEX
                } else {
                    &wf_ext as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX
                };

                let audio_client: IAudioClient = match unsafe { device.Activate(CLSCTX_ALL, None) }
                {
                    Ok(c) => c,
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };
                if unsafe {
                    audio_client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr, None)
                } != windows::core::HRESULT(0)
                {
                    continue;
                }

                let mut default_period = 0i64;
                let mut min_period = 0i64;
                unsafe {
                    let _ = audio_client
                        .GetDevicePeriod(Some(&mut default_period), Some(&mut min_period));
                }
                let mut buffer_duration = if min_period > 0 {
                    min_period
                } else if default_period > 0 {
                    default_period.max(100_000)
                } else {
                    100_000
                };

                let init_res = unsafe {
                    audio_client.Initialize(
                        AUDCLNT_SHAREMODE_EXCLUSIVE,
                        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                        buffer_duration,
                        buffer_duration,
                        wf_ptr,
                        None,
                    )
                };

                let active_client = match init_res {
                    Ok(()) => audio_client,
                    Err(e)
                        if e.code().0 == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED_HR
                            || e.code() == windows::core::HRESULT(0x80070057u32 as i32) =>
                    {
                        let aligned_frames = match unsafe { audio_client.GetBufferSize() } {
                            Ok(f) if f > 0 => f,
                            _ => {
                                last_err = anyhow::anyhow!(e);
                                continue;
                            }
                        };
                        buffer_duration = ((10_000_000.0 * aligned_frames as f64
                            / carrier_rate as f64)
                            + 0.5) as i64;
                        drop(audio_client);
                        let retry_client: IAudioClient =
                            match unsafe { device.Activate(CLSCTX_ALL, None) } {
                                Ok(c) => c,
                                Err(e2) => {
                                    last_err = anyhow::anyhow!(e2);
                                    continue;
                                }
                            };
                        if let Err(e2) = unsafe {
                            retry_client.Initialize(
                                AUDCLNT_SHAREMODE_EXCLUSIVE,
                                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                                buffer_duration,
                                buffer_duration,
                                wf_ptr,
                                None,
                            )
                        } {
                            last_err = anyhow::anyhow!(e2);
                            continue;
                        }
                        retry_client
                    }
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };

                let event =
                    OwnedHandle::new(match unsafe { CreateEventW(None, false, false, None) } {
                        Ok(ev) => ev,
                        Err(e) => {
                            last_err = anyhow::anyhow!(e);
                            continue;
                        }
                    });
                if let Err(e) = unsafe { active_client.SetEventHandle(event.as_raw()) } {
                    last_err = anyhow::anyhow!(e);
                    continue;
                }
                let buffer_frames = match unsafe { active_client.GetBufferSize() } {
                    Ok(f) => f,
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };
                let render_client: IAudioRenderClient = match unsafe { active_client.GetService() }
                {
                    Ok(rc) => rc,
                    Err(e) => {
                        last_err = anyhow::anyhow!(e);
                        continue;
                    }
                };

                return Ok((
                    active_client,
                    render_client,
                    event.into_raw(),
                    buffer_frames,
                    rate_factor,
                    label.to_string(),
                ));
            }
        }

        Err(last_err)
    }

    const SPATIAL_714_STATIC_OBJECT_TYPES: [AudioObjectType; 12] = [
        AudioObjectType_FrontLeft,
        AudioObjectType_FrontRight,
        AudioObjectType_FrontCenter,
        AudioObjectType_LowFrequency,
        AudioObjectType_SideLeft,
        AudioObjectType_SideRight,
        AudioObjectType_BackLeft,
        AudioObjectType_BackRight,
        AudioObjectType_TopFrontLeft,
        AudioObjectType_TopFrontRight,
        AudioObjectType_TopBackLeft,
        AudioObjectType_TopBackRight,
    ];

    // Field declaration order is load-bearing for COM teardown: child `ISpatialAudioObject`
    // instances must be released before `stream`, `_fmt_enum`, and `_spatial_client`.
    struct SpatialAudio714State {
        objects: Vec<ISpatialAudioObject>,
        stream: ISpatialAudioObjectRenderStream,
        _fmt_enum: IAudioFormatEnumerator,
        _spatial_client: ISpatialAudioClient,
        carrier_label: String,
    }

    impl SpatialAudio714State {
        fn activate_static_objects(&mut self) -> Result<()> {
            self.objects.clear();
            if self.objects.capacity() < SPATIAL_714_STATIC_OBJECT_TYPES.len() {
                self.objects
                    .reserve(SPATIAL_714_STATIC_OBJECT_TYPES.len() - self.objects.capacity());
            }
            for obj_type in SPATIAL_714_STATIC_OBJECT_TYPES {
                let obj =
                    unsafe { self.stream.ActivateSpatialAudioObject(obj_type) }.map_err(|e| {
                        anyhow::anyhow!("ActivateSpatialAudioObject({:?}) failed: {}", obj_type, e)
                    })?;
                self.objects.push(obj);
            }
            Ok(())
        }
    }

    /// Opens a 12-channel (`7.1.4` / `11.1`) Windows Spatial Audio (`ISpatialAudioClient` ->
    /// `ISpatialAudioObjectRenderStream`) stream with all 12 static 7.1.4 bed objects
    /// (`FL, FR, FC, LFE, SL, SR, BL, BR, TFL, TFR, TBL, TBR`).
    ///
    /// When "Dolby Atmos for Home Theater" (`DolbyMatEnc.dll`) is active on the endpoint
    /// (`GetMaxDynamicObjectCount() > 0`), Windows Spatial Audio encodes all 12 static bed objects
    /// into Dolby MAT 2.0 over HDMI/eARC so the AVR (e.g. Denon AVR-X3800H via LG OLED C4 eARC)
    /// receives discrete 7.1.4 (11.1) with height channels. If Windows Spatial Sound is Off
    /// (`GetMaxDynamicObjectCount() == 0`) and the Shared Mode `MixFormat` has `< 12` channels,
    /// `AudioDg.exe` would fold the 12 objects down to 2ch Stereo, so we decline and let WASAPI
    /// Exclusive 8ch (7.1) LPCM take over instead.
    fn try_open_spatial_audio_714_client(
        device: &IMMDevice,
        mix_channels: u16,
    ) -> Result<(SpatialAudio714State, HANDLE, u32, u32)> {
        use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
        use windows::Win32::System::Variant::VT_BLOB;
        use windows::core::Interface;

        let sac: ISpatialAudioClient = unsafe {
            device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| anyhow::anyhow!(e))?
        };
        let max_dyn = unsafe { sac.GetMaxDynamicObjectCount().unwrap_or(0) };
        if max_dyn == 0 && mix_channels < 12 {
            return Err(anyhow::anyhow!(
                "Windows Spatial Sound ('Dolby Atmos for Home Theater') is Off on endpoint (GetMaxDynamicObjectCount == 0, MixFormat == {}ch); declining ISpatialAudioClient to avoid stereo folddown",
                mix_channels
            ));
        }

        if let Err(e) = unsafe {
            sac.IsSpatialAudioStreamAvailable(&ISpatialAudioObjectRenderStream::IID, None)
        } {
            return Err(anyhow::anyhow!(
                "ISpatialAudioObjectRenderStream not available on endpoint: {}",
                e
            ));
        }

        let fmt_enum = unsafe {
            sac.GetSupportedAudioObjectFormatEnumerator()
                .map_err(|e| anyhow::anyhow!(e))?
        };
        let fmt_count = unsafe { fmt_enum.GetCount().unwrap_or(0) };
        if fmt_count == 0 {
            return Err(anyhow::anyhow!(
                "ISpatialAudioClient reported 0 supported AudioObject formats"
            ));
        }

        // Prefer 48000 Hz 32-bit float mono object format.
        let mut chosen_wf_ptr: *const WAVEFORMATEX = std::ptr::null();
        let mut chosen_rate = 48000u32;
        for idx in 0..fmt_count {
            if let Ok(wf_ptr) = unsafe { fmt_enum.GetFormat(idx) }
                && !wf_ptr.is_null()
            {
                let wf = unsafe { *wf_ptr };
                if wf.wBitsPerSample == 32 && wf.nChannels == 1 {
                    chosen_wf_ptr = wf_ptr;
                    chosen_rate = wf.nSamplesPerSec;
                    if wf.nSamplesPerSec == 48000 {
                        break;
                    }
                }
            }
        }
        if chosen_wf_ptr.is_null() {
            return Err(anyhow::anyhow!(
                "No 32-bit float mono AudioObject format found in ISpatialAudioClient"
            ));
        }

        let max_frames = unsafe {
            sac.GetMaxFrameCount(chosen_wf_ptr)
                .map_err(|e| anyhow::anyhow!(e))?
        };
        let event = OwnedHandle::new(unsafe {
            CreateEventW(None, false, false, None).map_err(|e| anyhow::anyhow!(e))?
        });

        let mask_714 = AudioObjectType_FrontLeft
            | AudioObjectType_FrontRight
            | AudioObjectType_FrontCenter
            | AudioObjectType_LowFrequency
            | AudioObjectType_SideLeft
            | AudioObjectType_SideRight
            | AudioObjectType_BackLeft
            | AudioObjectType_BackRight
            | AudioObjectType_TopFrontLeft
            | AudioObjectType_TopFrontRight
            | AudioObjectType_TopBackLeft
            | AudioObjectType_TopBackRight;

        let mut params = SpatialAudioObjectRenderStreamActivationParams {
            ObjectFormat: chosen_wf_ptr,
            StaticObjectTypeMask: mask_714,
            MinDynamicObjectCount: 0,
            MaxDynamicObjectCount: 0,
            Category: AudioCategory_Media,
            EventHandle: event.as_raw(),
            NotifyObject: std::mem::ManuallyDrop::new(None),
        };
        let mut pv = std::mem::ManuallyDrop::new(PROPVARIANT::default());
        unsafe {
            (*pv.Anonymous.Anonymous).vt = VT_BLOB;
            (*pv.Anonymous.Anonymous).Anonymous.blob = BLOB {
                cbSize: std::mem::size_of::<SpatialAudioObjectRenderStreamActivationParams>()
                    as u32,
                pBlobData: &mut params as *mut _ as *mut u8,
            };
        }

        let stream: ISpatialAudioObjectRenderStream =
            unsafe { sac.ActivateSpatialAudioStream(&*pv) }
                .map_err(|e| anyhow::anyhow!("ActivateSpatialAudioStream(7.1.4) failed: {}", e))?;

        let carrier_label = if max_dyn > 0 {
            format!(
                "ISpatialAudioClient 7.1.4 (Dolby Atmos / MAT 2.0, {} dynamic objects supported)",
                max_dyn
            )
        } else {
            "ISpatialAudioClient 7.1.4 (Static Bed — enable 'Dolby Atmos for Home Theater' in Windows Sound Settings for eARC heights)".to_string()
        };

        let mut spatial_state = SpatialAudio714State {
            objects: Vec::with_capacity(SPATIAL_714_STATIC_OBJECT_TYPES.len()),
            stream,
            _fmt_enum: fmt_enum,
            _spatial_client: sac,
            carrier_label,
        };
        // Activate all 12 static 7.1.4 bed objects in RustTracker's canonical 12-channel order:
        // [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:BL(RL), 7:BR(RR), 8:TFL, 9:TFR, 10:TBL(TRL), 11:TBR(TRR)]
        spatial_state.activate_static_objects()?;

        Ok((
            spatial_state,
            event.into_raw(),
            max_frames.max(480),
            chosen_rate,
        ))
    }

    fn try_open_shared_client(
        device: &IMMDevice,
        channels: u16,
        rate: u32,
        channel_mask: u32,
        fmt: WasapiPcmFormat,
    ) -> Result<(IAudioClient, IAudioRenderClient, HANDLE, u32)> {
        let wf = build_wave_format(channels, rate, channel_mask, fmt);
        let wf_ptr = &wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX;

        let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let mut default_period = 0i64;
        let mut min_period = 0i64;
        unsafe {
            let _ = audio_client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period));
        }
        let buffer_duration = if default_period > 0 {
            default_period.max(200_000)
        } else if min_period > 0 {
            min_period.max(200_000)
        } else {
            200_000
        };

        let stream_flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

        unsafe {
            audio_client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    buffer_duration,
                    0,
                    wf_ptr,
                    None,
                )
                .map_err(|e| anyhow::anyhow!(e))?;
        }

        let event = OwnedHandle::new(unsafe { CreateEventW(None, false, false, None)? });
        unsafe {
            audio_client
                .SetEventHandle(event.as_raw())
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        let buffer_frames = unsafe { audio_client.GetBufferSize()? };
        let render_client: IAudioRenderClient =
            unsafe { audio_client.GetService().map_err(|e| anyhow::anyhow!(e))? };
        Ok((audio_client, render_client, event.into_raw(), buffer_frames))
    }

    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_token_thread = stop_token.clone();
    let shared_state_thread = shared_state.clone();
    let selected_dev_owned = selected_device_name.map(|s| s.to_string());
    let (init_tx, init_rx) =
        bounded::<Result<(u16, u32, WasapiPcmFormat, String, bool, String)>>(1);
    let (pipe_tx, pipe_rx) = bounded::<(
        rtrb::Consumer<AudioChunk>,
        rtrb::Producer<AudioChunk>,
        Sender<DspMessage>,
        Sender<()>,
    )>(1);

    let thread_handle = std::thread::spawn(move || {
        let _com_guard = ComGuard::init_multithreaded();
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
        }

        let enumerator: IMMDeviceEnumerator =
            match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
                Ok(e) => e,
                Err(e) => {
                    let _ = init_tx.send(Err(anyhow::anyhow!(
                        "Failed to create MMDeviceEnumerator: {}",
                        e
                    )));
                    return;
                }
            };

        let mut candidate_devices: Vec<(IMMDevice, String)> = Vec::new();
        let push_unique_wasapi_dev = |list: &mut Vec<(IMMDevice, String)>, dev: IMMDevice| {
            let id = get_wasapi_device_id(&dev);
            let already = list
                .iter()
                .any(|(existing, _)| !id.is_empty() && get_wasapi_device_id(existing) == id);
            if !already {
                let name = get_wasapi_device_name(&dev).unwrap_or_else(|| "Unknown".to_string());
                list.push((dev, name));
            }
        };

        if let Some(ref target_name) = selected_dev_owned
            && let Ok(collection) =
                unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        {
            let count = unsafe { collection.GetCount().unwrap_or(0) };
            let target_lower = target_name.to_lowercase();
            for i in 0..count {
                if let Ok(dev) = unsafe { collection.Item(i) } {
                    let name =
                        get_wasapi_device_name(&dev).unwrap_or_else(|| "Unknown".to_string());
                    let name_lower = name.to_lowercase();
                    if name_lower == target_lower
                        || name_lower.contains(&target_lower)
                        || target_lower.contains(&name_lower)
                    {
                        push_unique_wasapi_dev(&mut candidate_devices, dev);
                    }
                }
            }
        }

        if selected_dev_owned.is_none() {
            if let Ok(default_dev) =
                unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            {
                push_unique_wasapi_dev(&mut candidate_devices, default_dev);
            }
            if candidate_devices.is_empty()
                && let Ok(collection) =
                    unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            {
                let count = unsafe { collection.GetCount().unwrap_or(0) };
                for i in 0..count {
                    if let Ok(dev) = unsafe { collection.Item(i) } {
                        let name =
                            get_wasapi_device_name(&dev).unwrap_or_else(|| "Unknown".to_string());
                        if is_relevant_audio_device(&name) {
                            push_unique_wasapi_dev(&mut candidate_devices, dev);
                        }
                    }
                }
            }
        }

        let channel_candidates: Vec<(u16, u32)> = vec![
            (12, 0x2D63F),
            (8, 0x63F),
            (8, 0xFF),
            (6, 0x3F),
            (6, 0x60F),
            (4, 0x33),
        ];

        let mut rate_candidates = Vec::with_capacity(3);
        for &r in &[target_rate.max(44100), 48000, 44100] {
            if !rate_candidates.contains(&r) {
                rate_candidates.push(r);
            }
        }

        let exclusive_fmt_candidates = [
            WasapiPcmFormat::I24In32,
            WasapiPcmFormat::F32,
            WasapiPcmFormat::I32,
            WasapiPcmFormat::I16,
        ];
        let shared_fmt_candidates = [
            WasapiPcmFormat::F32,
            WasapiPcmFormat::I24In32,
            WasapiPcmFormat::I32,
            WasapiPcmFormat::I16,
        ];

        let mut opened: Option<(
            IMMDevice,
            String,
            Option<(IAudioClient, IAudioRenderClient)>,
            HANDLE,
            u32,
            u16,
            u32,
            u32,
            WasapiPcmFormat,
            bool,
            Option<(crate::mpegh::Mpegh714Encoder, usize, String)>,
            Option<SpatialAudio714State>,
        )> = None;
        let mut last_err =
            anyhow::anyhow!("No multichannel WASAPI support detected on candidate endpoints");

        'dev_loop: for (dev, dev_name) in candidate_devices {
            let other_active_pid = device_has_other_active_audio_session(&dev);
            if let Some(pid) = other_active_pid {
                eprintln!(
                    "[audio] Endpoint '{}' has an active audio session from PID {}; sharing via Windows Spatial Audio / WASAPI Shared Mode (skipping Exclusive Mode).",
                    dev_name, pid
                );
            }

            let probe_client: Option<IAudioClient> = unsafe { dev.Activate(CLSCTX_ALL, None).ok() };
            let mix_channels = if let Some(ref pc) = probe_client {
                unsafe {
                    if let Ok(mix_ptr) = pc.GetMixFormat() {
                        let _mix_guard = CoTaskMemWaveFormat(mix_ptr);
                        (*mix_ptr).nChannels
                    } else {
                        2
                    }
                }
            } else {
                2
            };
            drop(probe_client);

            for &(ch, mask) in &channel_candidates {
                if ch == 12 && !mpegh_bitstream {
                    continue;
                }
                // 0. When 12-channel (7.1.4 / 11.1) output is enabled (`ch == 12 && mpegh_bitstream`),
                // first check if Windows Spatial Audio (`Dolby Atmos for Home Theater`, `GetMaxDynamicObjectCount() > 0`)
                // is active on the endpoint BEFORE probing `AUDCLNT_SHAREMODE_EXCLUSIVE` (since exclusive-mode
                // format/init calls can cause Windows `AudioDg.exe` to reset Spatial Sound to Off).
                // `ISpatialAudioClient` runs in Shared Mode inside `AudioDg.exe` alongside other desktop
                // apps (e.g. Chrome) without taking an Exclusive Mode hardware lock.
                if ch == 12 && mpegh_bitstream {
                    match try_open_spatial_audio_714_client(&dev, mix_channels) {
                        Ok((spatial, ev, max_frames, spatial_rate)) => {
                            eprintln!(
                                "[audio] Windows Spatial Audio 7.1.4 (11.1) negotiated on '{}': 12 static bed objects @ {} Hz [{}] (max {} frames/period)",
                                dev_name, spatial_rate, spatial.carrier_label, max_frames
                            );
                            opened = Some((
                                dev,
                                dev_name,
                                None,
                                ev,
                                max_frames,
                                12,
                                0x2D63F,
                                spatial_rate,
                                WasapiPcmFormat::F32,
                                false,
                                None,
                                Some(spatial),
                            ));
                            break 'dev_loop;
                        }
                        Err(e) => {
                            last_err = e;
                        }
                    }
                }

                for &rate in &rate_candidates {
                    // 1. Primary LPCM path: WASAPI Shared Mode whenever the Windows Shared Mode MixFormat
                    // itself has >= `ch` discrete channels (so AudioDg.exe outputs all `ch` channels natively
                    // without Exclusive Mode HDMI InfoFrame renegotiation or locking out other apps).
                    if mix_channels >= ch {
                        for &fmt in &shared_fmt_candidates {
                            match try_open_shared_client(&dev, ch, rate, mask, fmt) {
                                Ok((ac, rc, ev, buf_frames)) => {
                                    eprintln!(
                                        "[audio] WASAPI Shared Multichannel LPCM negotiated on '{}': {}ch (mask 0x{:X}) @ {} Hz [{}] (buffer {} frames)",
                                        dev_name,
                                        ch,
                                        mask,
                                        rate,
                                        fmt.label(),
                                        buf_frames
                                    );
                                    opened = Some((
                                        dev,
                                        dev_name,
                                        Some((ac, rc)),
                                        ev,
                                        buf_frames,
                                        ch,
                                        mask,
                                        rate,
                                        fmt,
                                        false,
                                        None,
                                        None,
                                    ));
                                    break 'dev_loop;
                                }
                                Err(e) => {
                                    last_err = e;
                                }
                            }
                        }
                    }

                    // 2. Fallback path: WASAPI Exclusive Mode LPCM (up to 8ch; standard HDMI LPCM maxes out
                    // at 8 channels, so skip `ch > 8` to avoid hammering the GPU HDMI driver with unsupported
                    // 12ch Exclusive LPCM probes). Only probe Exclusive Mode when `allow_exclusive` is true
                    // and no other process has an active audio session on the endpoint.
                    if allow_exclusive && other_active_pid.is_none() && mix_channels < ch && ch <= 8
                    {
                        for &fmt in &exclusive_fmt_candidates {
                            match try_open_exclusive_client(&dev, ch, rate, mask, fmt) {
                                Ok((ac, rc, ev, buf_frames)) => {
                                    eprintln!(
                                        "[audio] WASAPI Exclusive Multichannel LPCM negotiated on '{}': {}ch (mask 0x{:X}) @ {} Hz [{}] (buffer {} frames)",
                                        dev_name,
                                        ch,
                                        mask,
                                        rate,
                                        fmt.label(),
                                        buf_frames
                                    );
                                    opened = Some((
                                        dev,
                                        dev_name,
                                        Some((ac, rc)),
                                        ev,
                                        buf_frames,
                                        ch,
                                        mask,
                                        rate,
                                        fmt,
                                        true,
                                        None,
                                        None,
                                    ));
                                    break 'dev_loop;
                                }
                                Err(e) => {
                                    last_err = e;
                                }
                            }
                        }
                    }
                }

                // 3. Direct HDMI AVR fallback: probe native EDID IEC 61937-13 MPEG-H 3D Audio (CICP Layout 19)
                // when connected directly to an HDMI sink that advertises MPEG-H (`0x00000019` / `0x0000000f`).
                if allow_exclusive && other_active_pid.is_none() && ch == 12 && mpegh_bitstream {
                    match try_open_exclusive_mpegh_iec61937_client(&dev, false) {
                        Ok((ac, rc, ev, buf_frames, rate_factor, carrier_label)) => {
                            match crate::mpegh::Mpegh714Encoder::new(rate_factor as u8, true) {
                                Ok(enc) => {
                                    let carrier_frame_bytes = rate_factor;
                                    eprintln!(
                                        "[audio] WASAPI Exclusive MPEG-H 3D Audio 7.1.4 (11.1, CICP 19) negotiated on '{}': 12ch @ {} Hz via {} (buffer {} carrier frames)",
                                        dev_name,
                                        crate::mpegh::MPEGH_SAMPLE_RATE,
                                        carrier_label,
                                        buf_frames
                                    );
                                    opened = Some((
                                        dev,
                                        dev_name,
                                        Some((ac, rc)),
                                        ev,
                                        buf_frames,
                                        12,
                                        0x2D63F,
                                        crate::mpegh::MPEGH_SAMPLE_RATE,
                                        WasapiPcmFormat::I16,
                                        true,
                                        Some((enc, carrier_frame_bytes, carrier_label)),
                                        None,
                                    ));
                                    break 'dev_loop;
                                }
                                Err(e) => {
                                    unsafe {
                                        let _ = CloseHandle(ev);
                                    }
                                    drop(rc);
                                    drop(ac);
                                    last_err = e;
                                }
                            }
                        }
                        Err(e) => {
                            last_err = e;
                        }
                    }
                }
            }
        }

        // Declare `event` before WASAPI / Spatial Audio COM interfaces so reverse-declaration-order
        // unwinding always drops COM clients/streams before closing the Win32 event handle.
        let mut event = OwnedHandle::new(HANDLE::default());
        let (
            active_device,
            active_device_name,
            mut wasapi_clients,
            raw_event,
            mut buffer_frames,
            negotiated_channels,
            negotiated_mask,
            negotiated_rate,
            negotiated_fmt,
            is_exclusive,
            mut mpegh_state,
            mut spatial_state,
        ) = match opened {
            Some(v) => v,
            None => {
                let _ = init_tx.send(Err(last_err));
                return;
            }
        };
        event.reset(raw_event);

        let is_mpegh_iec_bitstream = mpegh_state.is_some();
        let is_spatial_714 = spatial_state.is_some();
        let is_11_1_active = is_mpegh_iec_bitstream || is_spatial_714;
        let mpegh_carrier_label = if let Some((_, _, ref label)) = mpegh_state {
            label.clone()
        } else if let Some(ref spatial) = spatial_state {
            spatial.carrier_label.clone()
        } else {
            String::new()
        };

        if init_tx
            .send(Ok((
                negotiated_channels,
                negotiated_rate,
                negotiated_fmt,
                active_device_name.clone(),
                is_11_1_active,
                mpegh_carrier_label,
            )))
            .is_err()
        {
            return;
        }

        let (mut ready_rx, mut free_tx, tx, _shutdown_guard) = match pipe_rx.recv() {
            Ok(pipes) => pipes,
            Err(_) => {
                return;
            }
        };

        let hardware_channels = negotiated_channels as usize;
        let bytes_per_sample = negotiated_fmt.bytes_per_sample();
        let frame_bytes = if let Some((_, carrier_fb, _)) = mpegh_state.as_ref() {
            *carrier_fb
        } else {
            hardware_channels * bytes_per_sample
        };
        let window_size = calculate_power_of_two_window_size(negotiated_rate);

        let mut fft_buffer = vec![0.0f32; window_size];
        let mut channel_fft_buffers = vec![vec![0.0f32; window_size]; hardware_channels.max(12)];
        let mut fft_index = 0usize;
        let recycle_rx = dsp_recycle_channel().1.clone();
        let dsp_send_interval = (negotiated_rate as usize / 120).max(1);
        let mut frames_since_last_dsp_send = dsp_send_interval;

        let mut last_channel_vus = Vec::new();
        let mut last_spatial_channels = 2usize;
        let mut last_current_order = 0;
        let mut last_current_row = 0;
        let mut last_bpm = 0;
        let mut last_speed = 0;
        let mut last_current_seconds = 0.0;
        let mut last_current_row_string = String::new();
        let mut last_seek_epoch = 0u64;
        let mut upmixer_dsp = SurroundUpmixerDsp::new(negotiated_rate);
        let mut upmix_peak_vus = [0.0f32; 12];
        let mut current_chunk: Option<AudioChunk> = None;
        let mut chunk_frame_pos = 0usize;
        let mut warmup_silent_buffers =
            if is_exclusive && !is_mpegh_iec_bitstream && !is_spatial_714 {
                1u32
            } else {
                0u32
            };
        let startup_fade_frames = ((negotiated_rate as usize) * 35 / 1000).max(64);
        let mut startup_fade_pos = 0usize;
        let mut iec_byte_queue: std::collections::VecDeque<u8> = if is_mpegh_iec_bitstream {
            std::collections::VecDeque::with_capacity(65536 * 2)
        } else {
            std::collections::VecDeque::new()
        };
        let mut consecutive_render_errors = 0u32;

        let is_fatal_device_error = |hr: i32| -> bool {
            const AUDCLNT_E_DEVICE_INVALIDATED_HR: i32 = 0x88890004u32 as i32;
            const AUDCLNT_E_RESOURCES_INVALIDATED_HR: i32 = 0x88890026u32 as i32;
            const SPTLAUDCLNT_E_STREAM_NOT_AVAILABLE_HR: i32 = 0x88890106u32 as i32;
            const SPTLAUDCLNT_E_INTERNAL_HR: i32 = 0x8889010Du32 as i32;
            matches!(
                hr,
                AUDCLNT_E_DEVICE_INVALIDATED_HR
                    | AUDCLNT_E_RESOURCES_INVALIDATED_HR
                    | SPTLAUDCLNT_E_STREAM_NOT_AVAILABLE_HR
                    | SPTLAUDCLNT_E_INTERNAL_HR
            )
        };

        let (mut latest_seek_epoch, rt_control) = {
            let mut state = shared_state_thread.lock().unwrap();
            state.sync_rt_control();
            (state.seek_epoch, Arc::clone(&state.rt_control))
        };
        let is_paused_init = rt_control.is_paused();
        let mut stream_started = !is_mpegh_iec_bitstream && !is_paused_init;
        let mut fatal_err_msg: Option<String> = None;
        let mut logged_reacquire_wait = false;

        // Start the hardware stream:
        // - For ISpatialAudioObjectRenderStream, call `stream.Start()` before entering the event loop.
        // - For LPCM WASAPI, prime the buffer with silence before calling `audio_client.Start()`.
        // - For IEC 61937-13 MPEG-H bitstreaming, pre-fill the first DMA buffer with real IEC 61937
        //   bursts inside the loop before calling `audio_client.Start()`.
        let mut start_failed = false;
        if !is_paused_init {
            if let Some(ref spatial) = spatial_state {
                unsafe {
                    if let Err(e) = spatial.stream.Start() {
                        fatal_err_msg = Some(format!(
                            "[audio] ISpatialAudioObjectRenderStream::Start() failed: {:?}",
                            e
                        ));
                        rt_control.signal_audio_device_lost();
                        start_failed = true;
                    }
                }
            } else if !is_mpegh_iec_bitstream
                && let Some((ref audio_client, ref render_client)) = wasapi_clients
            {
                unsafe {
                    let padding = if is_exclusive {
                        0
                    } else {
                        audio_client.GetCurrentPadding().unwrap_or(0)
                    };
                    let prime_frames = buffer_frames.saturating_sub(padding);
                    if prime_frames > 0
                        && let Ok(buf_ptr) = render_client.GetBuffer(prime_frames)
                    {
                        std::ptr::write_bytes(buf_ptr, 0, (prime_frames as usize) * frame_bytes);
                        let _ = render_client
                            .ReleaseBuffer(prime_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                    }
                    if let Err(e) = audio_client.Start() {
                        fatal_err_msg = Some(format!("[audio] WASAPI Start() failed: {:?}", e));
                        rt_control.signal_audio_device_lost();
                        start_failed = true;
                    }
                }
            }
        }

        let mut consecutive_wait_timeouts = 0u32;

        while !start_failed && !stop_token_thread.load(std::sync::atomic::Ordering::Relaxed) {
            let is_paused = rt_control.is_paused();
            let ctrl_epoch = rt_control.seek_epoch();
            if ctrl_epoch > latest_seek_epoch {
                latest_seek_epoch = ctrl_epoch;
            }
            let upmix_cfg = rt_control.load_upmix_config();

            if latest_seek_epoch > last_seek_epoch {
                last_seek_epoch = latest_seek_epoch;
                if let Some(stale) = current_chunk.take() {
                    let _ = free_tx.push(stale);
                }
                chunk_frame_pos = 0;
                iec_byte_queue.clear();
                if let Some((ref mut mpegh_encoder, _, _)) = mpegh_state {
                    mpegh_encoder.clear_accumulator();
                }
            } else if let Some(ref c) = current_chunk
                && c.seek_epoch < latest_seek_epoch
            {
                let stale = current_chunk.take().unwrap();
                let _ = free_tx.push(stale);
                chunk_frame_pos = 0;
            }
            flush_stale_ready_chunks(&mut ready_rx, &mut free_tx, latest_seek_epoch);

            // Stop active hardware stream / release Exclusive Mode lock while paused so we never
            // pump 100 Hz spatial objects or hold an Exclusive lock when playback is paused.
            if is_paused {
                consecutive_wait_timeouts = 0;
                if stream_started {
                    fft_buffer.fill(0.0);
                    for ch_buf in &mut channel_fft_buffers {
                        ch_buf.fill(0.0);
                    }
                }
                if let Some(ref mut spatial) = spatial_state {
                    if stream_started {
                        unsafe {
                            let _ = spatial.stream.Stop();
                        }
                        // Calling `Reset()` revokes all active `ISpatialAudioObject` instances, so
                        // release them beforehand and re-activate on resume inside
                        // `BeginUpdatingAudioObjects`.
                        spatial.objects.clear();
                        unsafe {
                            let _ = spatial.stream.Reset();
                        }
                        stream_started = false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    continue;
                } else if is_exclusive {
                    if let Some((ac, rc)) = wasapi_clients.take() {
                        unsafe {
                            let _ = ac.Stop();
                            let _ = ac.Reset();
                        }
                        drop(rc);
                        drop(ac);
                        event.clear();
                        stream_started = false;
                        logged_reacquire_wait = false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    continue;
                }
            }

            // Resume from pause: restart Spatial Audio stream or re-acquire Exclusive Mode lock.
            if !is_paused {
                if let Some(ref spatial) = spatial_state {
                    if !stream_started {
                        startup_fade_pos = 0;
                        consecutive_wait_timeouts = 0;
                        unsafe {
                            if let Err(e) = spatial.stream.Start() {
                                fatal_err_msg = Some(format!(
                                    "[audio] ISpatialAudioObjectRenderStream::Start() on resume failed: {:?}",
                                    e
                                ));
                                rt_control.signal_audio_device_lost();
                                break;
                            }
                        }
                        stream_started = true;
                    }
                } else if is_exclusive && wasapi_clients.is_none() {
                    if is_mpegh_iec_bitstream {
                        match try_open_exclusive_mpegh_iec61937_client(&active_device, false) {
                            Ok((new_ac, new_rc, new_ev, new_frames, _, _)) => {
                                buffer_frames = new_frames;
                                event.reset(new_ev);
                                wasapi_clients = Some((new_ac, new_rc));
                                iec_byte_queue.clear();
                                if let Some((ref mut mpegh_encoder, _, _)) = mpegh_state {
                                    mpegh_encoder.clear_accumulator();
                                }
                                startup_fade_pos = 0;
                                stream_started = false;
                                logged_reacquire_wait = false;
                            }
                            Err(e) => {
                                if !logged_reacquire_wait {
                                    eprintln!(
                                        "[audio] Waiting to re-acquire WASAPI Exclusive MPEG-H device: {:?}",
                                        e
                                    );
                                    logged_reacquire_wait = true;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(100));
                                continue;
                            }
                        }
                    } else {
                        match try_open_exclusive_client(
                            &active_device,
                            negotiated_channels,
                            negotiated_rate,
                            negotiated_mask,
                            negotiated_fmt,
                        ) {
                            Ok((new_ac, new_rc, new_ev, new_frames)) => {
                                buffer_frames = new_frames;
                                event.reset(new_ev);
                                startup_fade_pos = 0;
                                warmup_silent_buffers = 1;
                                unsafe {
                                    if let Ok(buf_ptr) = new_rc.GetBuffer(buffer_frames) {
                                        std::ptr::write_bytes(
                                            buf_ptr,
                                            0,
                                            (buffer_frames as usize) * frame_bytes,
                                        );
                                        let _ = new_rc.ReleaseBuffer(
                                            buffer_frames,
                                            AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
                                        );
                                    }
                                    let _ = new_ac.Start();
                                }
                                wasapi_clients = Some((new_ac, new_rc));
                                stream_started = true;
                                logged_reacquire_wait = false;
                            }
                            Err(e) => {
                                if !logged_reacquire_wait {
                                    eprintln!(
                                        "[audio] Waiting to re-acquire WASAPI Exclusive LPCM device: {:?}",
                                        e
                                    );
                                    logged_reacquire_wait = true;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(100));
                                continue;
                            }
                        }
                    }
                }
            }

            if stream_started {
                let wait_res = unsafe { WaitForSingleObject(event.as_raw(), 50) };
                if wait_res.0 != WAIT_OBJECT_0 {
                    if !is_paused {
                        consecutive_wait_timeouts += 1;
                        if consecutive_wait_timeouts >= 60 {
                            fatal_err_msg = Some(
                                "[audio] WASAPI/Spatial Audio event timed out for ~3s while playing; triggering device recovery"
                                    .to_string(),
                            );
                            rt_control.signal_audio_device_lost();
                            break;
                        }
                    }
                    continue;
                }
                consecutive_wait_timeouts = 0;
            }

            let mut frames_written = 0usize;
            let upmix_active = upmix_cfg.is_active_for_hardware(hardware_channels as i32);

            if let Some(ref mut spatial) = spatial_state {
                let mut dyn_count = 0u32;
                let mut avail_frames = 0u32;
                if let Err(e) = unsafe {
                    spatial
                        .stream
                        .BeginUpdatingAudioObjects(&mut dyn_count, &mut avail_frames)
                } {
                    consecutive_render_errors += 1;
                    if is_fatal_device_error(e.code().0) || consecutive_render_errors >= 8 {
                        fatal_err_msg = Some(format!(
                            "[audio] ISpatialAudioObjectRenderStream::BeginUpdatingAudioObjects failed: {:?}",
                            e
                        ));
                        rt_control.signal_audio_device_lost();
                        break;
                    }
                    continue;
                }
                if avail_frames == 0 {
                    unsafe {
                        let _ = spatial.stream.EndUpdatingAudioObjects();
                    }
                    continue;
                }

                if spatial.objects.len() < SPATIAL_714_STATIC_OBJECT_TYPES.len()
                    && let Err(e) = spatial.activate_static_objects()
                {
                    unsafe {
                        let _ = spatial.stream.EndUpdatingAudioObjects();
                    }
                    spatial.objects.clear();
                    consecutive_render_errors += 1;
                    if consecutive_render_errors >= 8 {
                        fatal_err_msg = Some(format!(
                            "[audio] Failed to re-activate Spatial Audio 7.1.4 static objects: {:?}",
                            e
                        ));
                        rt_control.signal_audio_device_lost();
                        break;
                    }
                    continue;
                }

                let frames_to_render = avail_frames as usize;
                let mut obj_ptrs = [std::ptr::null_mut::<f32>(); 12];
                let mut buffer_err = false;
                let mut buffer_err_code: Option<i32> = None;
                for (c, obj) in spatial.objects.iter().enumerate().take(12) {
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    let mut byte_len = 0u32;
                    match unsafe { obj.GetBuffer(&mut ptr, &mut byte_len) } {
                        Ok(()) if !ptr.is_null() && (byte_len as usize) >= frames_to_render * 4 => {
                            obj_ptrs[c] = ptr as *mut f32;
                        }
                        Ok(()) => {
                            buffer_err = true;
                            break;
                        }
                        Err(e) => {
                            buffer_err = true;
                            buffer_err_code = Some(e.code().0);
                            break;
                        }
                    }
                }
                if buffer_err {
                    unsafe {
                        let _ = spatial.stream.EndUpdatingAudioObjects();
                    }
                    spatial.objects.clear();
                    consecutive_render_errors += 1;
                    if buffer_err_code.is_some_and(is_fatal_device_error)
                        || consecutive_render_errors >= 8
                    {
                        fatal_err_msg = Some(format!(
                            "[audio] ISpatialAudioObject::GetBuffer failed (hr={:?}); triggering device recovery",
                            buffer_err_code
                        ));
                        rt_control.signal_audio_device_lost();
                        break;
                    }
                    continue;
                }

                if upmix_active {
                    upmixer_dsp.sync_config(&upmix_cfg, negotiated_rate);
                }

                while frames_written < frames_to_render {
                    if current_chunk.is_none() {
                        match ready_rx.pop() {
                            Ok(chunk) => {
                                if chunk.seek_epoch < latest_seek_epoch {
                                    let _ = free_tx.push(chunk);
                                    continue;
                                }
                                last_channel_vus.clear();
                                last_channel_vus.extend_from_slice(&chunk.channel_vus);
                                last_spatial_channels = (chunk.spatial_channels as usize).max(1);
                                last_current_order = chunk.current_order;
                                last_current_row = chunk.current_row;
                                last_bpm = chunk.bpm;
                                last_speed = chunk.speed;
                                last_current_seconds = chunk.current_seconds;
                                last_current_row_string.clear();
                                last_current_row_string.push_str(&chunk.current_row_string);
                                last_seek_epoch = chunk.seek_epoch;
                                if chunk.seek_epoch > latest_seek_epoch {
                                    latest_seek_epoch = chunk.seek_epoch;
                                }

                                if chunk.track_ended {
                                    rt_control.signal_track_ended();
                                }

                                current_chunk = Some(chunk);
                                chunk_frame_pos = 0;
                            }
                            Err(_) => {
                                let rem = frames_to_render - frames_written;
                                unsafe {
                                    for &ptr in &obj_ptrs {
                                        std::ptr::write_bytes(ptr.add(frames_written), 0, rem);
                                    }
                                }
                                break;
                            }
                        }
                    }

                    if let Some(chunk) = current_chunk.as_mut() {
                        let frames_available = chunk.valid_frames.saturating_sub(chunk_frame_pos);
                        let frames_needed = frames_to_render - frames_written;

                        if frames_available == 0 {
                            let c = current_chunk.take().unwrap();
                            let _ = free_tx.push(c);
                            continue;
                        }

                        let to_copy = std::cmp::min(frames_available, frames_needed);
                        let src_ch = (chunk.spatial_channels as usize).max(1);
                        let target_upmix =
                            upmix_cfg.is_active_for(src_ch as i32, hardware_channels as i32);

                        if upmixer_dsp.is_blending_or_active(target_upmix) {
                            for i in 0..to_copy {
                                let src_frame_idx = chunk_frame_pos + i;
                                let dst_frame_idx = frames_written + i;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };
                                let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                    ..(src_frame_idx + 1) * hardware_channels];
                                let out_channels = upmixer_dsp
                                    .process_multichannel_frame_for_hardware_blended(
                                        frame_slice,
                                        src_ch,
                                        &upmix_cfg,
                                        hardware_channels,
                                        target_upmix,
                                    );

                                // `spatial.objects` are activated in RustTracker's canonical 12-channel order:
                                // [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:BL(RL), 7:BR(RR), 8:TFL, 9:TFR, 10:TBL(TRL), 11:TBR(TRR)].
                                for c in 0..12 {
                                    let s = (out_channels[c] * fade_gain).clamp(-1.0, 1.0);
                                    unsafe {
                                        *obj_ptrs[c].add(dst_frame_idx) = s;
                                    }
                                    upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                    channel_fft_buffers[c][fft_index] = s;
                                }

                                let mono = compute_fft_mono_sample(frame_slice, src_ch) * fade_gain;
                                fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                fft_index = (fft_index + 1) % window_size;
                            }
                        } else {
                            for i in 0..to_copy {
                                let src_frame_idx = chunk_frame_pos + i;
                                let dst_frame_idx = frames_written + i;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };

                                let mut mono = 0.0f32;
                                for c in 0..12 {
                                    let sample = if c < src_ch && c < hardware_channels {
                                        let src_c = if src_ch >= 8 {
                                            match c {
                                                4 => 6, // Canonical SL <- 0x63F slot 6 (SL)
                                                5 => 7, // Canonical SR <- 0x63F slot 7 (SR)
                                                6 => 4, // Canonical BL <- 0x63F slot 4 (BL)
                                                7 => 5, // Canonical BR <- 0x63F slot 5 (BR)
                                                _ => c,
                                            }
                                        } else {
                                            c
                                        };
                                        (chunk.samples[src_frame_idx * hardware_channels + src_c]
                                            * fade_gain)
                                            .clamp(-1.0, 1.0)
                                    } else {
                                        0.0
                                    };
                                    unsafe {
                                        *obj_ptrs[c].add(dst_frame_idx) = sample;
                                    }
                                    let ch_weight = match c {
                                        0 | 1 => 0.5,
                                        2 => std::f32::consts::FRAC_1_SQRT_2,
                                        _ => 0.25,
                                    };
                                    mono += sample * ch_weight;
                                    channel_fft_buffers[c][fft_index] = sample;
                                }

                                fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                fft_index = (fft_index + 1) % window_size;
                            }
                        }

                        chunk_frame_pos += to_copy;
                        frames_written += to_copy;
                    }
                }

                if let Err(e) = unsafe { spatial.stream.EndUpdatingAudioObjects() } {
                    consecutive_render_errors += 1;
                    if is_fatal_device_error(e.code().0) || consecutive_render_errors >= 8 {
                        fatal_err_msg = Some(format!(
                            "[audio] ISpatialAudioObjectRenderStream::EndUpdatingAudioObjects failed: {:?}",
                            e
                        ));
                        rt_control.signal_audio_device_lost();
                        break;
                    }
                    continue;
                }
                consecutive_render_errors = 0;
            } else if let Some((ref audio_client, ref render_client)) = wasapi_clients {
                let avail_frames = if is_exclusive {
                    buffer_frames
                } else {
                    let padding = match unsafe { audio_client.GetCurrentPadding() } {
                        Ok(p) => p,
                        Err(e) => {
                            consecutive_render_errors += 1;
                            if is_fatal_device_error(e.code().0) || consecutive_render_errors >= 8 {
                                fatal_err_msg = Some(format!(
                                    "[audio] WASAPI Shared GetCurrentPadding failed (device disconnected?): {:?}",
                                    e
                                ));
                                rt_control.signal_audio_device_lost();
                                break;
                            }
                            continue;
                        }
                    };
                    buffer_frames.saturating_sub(padding)
                };
                if avail_frames == 0 {
                    continue;
                }

                if is_paused || warmup_silent_buffers > 0 {
                    if is_paused {
                        startup_fade_pos = 0;
                    } else {
                        warmup_silent_buffers -= 1;
                    }
                    if let Ok(buf_ptr) = unsafe { render_client.GetBuffer(avail_frames) } {
                        unsafe {
                            std::ptr::write_bytes(
                                buf_ptr,
                                0,
                                (avail_frames as usize) * frame_bytes,
                            );
                            let _ = render_client
                                .ReleaseBuffer(avail_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                            if !stream_started {
                                if let Err(e) = audio_client.Start() {
                                    fatal_err_msg =
                                        Some(format!("[audio] WASAPI Start() failed: {:?}", e));
                                    rt_control.signal_audio_device_lost();
                                    break;
                                }
                                stream_started = true;
                            }
                        }
                    }
                    continue;
                }

                let buf_ptr = match unsafe { render_client.GetBuffer(avail_frames) } {
                    Ok(ptr) => ptr,
                    Err(e) => {
                        consecutive_render_errors += 1;
                        if is_fatal_device_error(e.code().0) || consecutive_render_errors >= 8 {
                            fatal_err_msg = Some(format!(
                                "[audio] WASAPI GetBuffer failed (device disconnected?): {:?}",
                                e
                            ));
                            rt_control.signal_audio_device_lost();
                            break;
                        }
                        continue;
                    }
                };

                let frames_to_render = avail_frames as usize;
                let out_bytes = unsafe {
                    std::slice::from_raw_parts_mut(buf_ptr, frames_to_render * frame_bytes)
                };

                if upmix_active {
                    upmixer_dsp.sync_config(&upmix_cfg, negotiated_rate);
                }

                if let Some((ref mut mpegh_encoder, _, _)) = mpegh_state {
                    // Real-time 12-channel (7.1.4 / 11.1, CICP Layout 19) MPEG-H 3D Audio -> IEC 61937-13
                    // HDMI bitstream encoding path. Each 1024-sample @ 48 kHz audio frame produces one
                    // IEC 61937-13 burst covering 1024 * rate_factor carrier frames @ 192 kHz.
                    let needed_iec_bytes = out_bytes.len();
                    while iec_byte_queue.len() < needed_iec_bytes {
                        if current_chunk.is_none() {
                            let timeout = if !stream_started {
                                std::time::Duration::from_millis(100)
                            } else {
                                std::time::Duration::from_millis(15)
                            };
                            match pop_ready_with_timeout(
                                &mut ready_rx,
                                &mut free_tx,
                                &stop_token_thread,
                                latest_seek_epoch,
                                timeout,
                            ) {
                                Some(chunk) => {
                                    if chunk.seek_epoch < latest_seek_epoch {
                                        let _ = free_tx.push(chunk);
                                        continue;
                                    }
                                    last_channel_vus.clear();
                                    last_channel_vus.extend_from_slice(&chunk.channel_vus);
                                    last_spatial_channels =
                                        (chunk.spatial_channels as usize).max(1);
                                    last_current_order = chunk.current_order;
                                    last_current_row = chunk.current_row;
                                    last_bpm = chunk.bpm;
                                    last_speed = chunk.speed;
                                    last_current_seconds = chunk.current_seconds;
                                    last_current_row_string.clear();
                                    last_current_row_string.push_str(&chunk.current_row_string);
                                    last_seek_epoch = chunk.seek_epoch;
                                    if chunk.seek_epoch > latest_seek_epoch {
                                        latest_seek_epoch = chunk.seek_epoch;
                                    }

                                    if chunk.track_ended {
                                        rt_control.signal_track_ended();
                                    }

                                    current_chunk = Some(chunk);
                                    chunk_frame_pos = 0;
                                }
                                None => {
                                    break;
                                }
                            }
                        }

                        if let Some(chunk) = current_chunk.as_mut() {
                            let frames_available =
                                chunk.valid_frames.saturating_sub(chunk_frame_pos);
                            if frames_available == 0 {
                                let c = current_chunk.take().unwrap();
                                let _ = free_tx.push(c);
                                continue;
                            }

                            let src_ch = (chunk.spatial_channels as usize).max(1);
                            let target_upmix =
                                upmix_cfg.is_active_for(src_ch as i32, hardware_channels as i32);
                            let use_upmixer = upmixer_dsp.is_blending_or_active(target_upmix);

                            for _ in 0..frames_available {
                                let src_frame_idx = chunk_frame_pos;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };
                                let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                    ..(src_frame_idx + 1) * hardware_channels];

                                if use_upmixer {
                                    let mut out_channels = upmixer_dsp
                                        .process_multichannel_frame_for_hardware_blended(
                                            frame_slice,
                                            src_ch,
                                            &upmix_cfg,
                                            hardware_channels,
                                            target_upmix,
                                        );
                                    for s in out_channels.iter_mut() {
                                        *s = (*s * fade_gain).clamp(-1.0, 1.0);
                                    }
                                    for c in 0..12 {
                                        let s = out_channels[c];
                                        upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                        channel_fft_buffers[c][fft_index] = s;
                                    }
                                    let mono =
                                        compute_fft_mono_sample(frame_slice, src_ch) * fade_gain;
                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;

                                    let _ = mpegh_encoder.push_rusttracker_714_frame(
                                        &out_channels,
                                        &mut iec_byte_queue,
                                    );
                                } else {
                                    let mut rt_frame = [0.0f32; 12];
                                    for c in 0..12 {
                                        let sample = if c < hardware_channels {
                                            (frame_slice[c] * fade_gain).clamp(-1.0, 1.0)
                                        } else {
                                            0.0
                                        };
                                        rt_frame[c] = sample;
                                    }
                                    if src_ch >= 8 {
                                        rt_frame.swap(4, 6);
                                        rt_frame.swap(5, 7);
                                    }
                                    let mut mono = 0.0f32;
                                    for c in 0..12 {
                                        let sample = rt_frame[c];
                                        let ch_weight = match c {
                                            0 | 1 => 0.5,
                                            2 => std::f32::consts::FRAC_1_SQRT_2,
                                            _ => 0.25,
                                        };
                                        mono += sample * ch_weight;
                                        channel_fft_buffers[c][fft_index] = sample;
                                    }
                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;

                                    let _ = mpegh_encoder
                                        .push_rusttracker_714_frame(&rt_frame, &mut iec_byte_queue);
                                }

                                chunk_frame_pos += 1;
                                frames_written += 1;
                                if iec_byte_queue.len() >= needed_iec_bytes {
                                    break;
                                }
                            }

                            if chunk_frame_pos >= chunk.valid_frames {
                                let c = current_chunk.take().unwrap();
                                let _ = free_tx.push(c);
                            }
                        }
                    }

                    let copy_bytes = iec_byte_queue.len().min(out_bytes.len());
                    for b in &mut out_bytes[..copy_bytes] {
                        *b = iec_byte_queue.pop_front().unwrap_or(0);
                    }
                    if copy_bytes < out_bytes.len() {
                        out_bytes[copy_bytes..].fill(0);
                    }
                } else {
                    while frames_written < frames_to_render {
                        if current_chunk.is_none() {
                            match ready_rx.pop() {
                                Ok(chunk) => {
                                    if chunk.seek_epoch < latest_seek_epoch {
                                        let _ = free_tx.push(chunk);
                                        continue;
                                    }
                                    last_channel_vus.clear();
                                    last_channel_vus.extend_from_slice(&chunk.channel_vus);
                                    last_spatial_channels =
                                        (chunk.spatial_channels as usize).max(1);
                                    last_current_order = chunk.current_order;
                                    last_current_row = chunk.current_row;
                                    last_bpm = chunk.bpm;
                                    last_speed = chunk.speed;
                                    last_current_seconds = chunk.current_seconds;
                                    last_current_row_string.clear();
                                    last_current_row_string.push_str(&chunk.current_row_string);
                                    last_seek_epoch = chunk.seek_epoch;
                                    if chunk.seek_epoch > latest_seek_epoch {
                                        latest_seek_epoch = chunk.seek_epoch;
                                    }

                                    if chunk.track_ended {
                                        rt_control.signal_track_ended();
                                    }

                                    current_chunk = Some(chunk);
                                    chunk_frame_pos = 0;
                                }
                                Err(_) => {
                                    out_bytes[frames_written * frame_bytes..].fill(0);
                                    break;
                                }
                            }
                        }

                        if let Some(chunk) = current_chunk.as_mut() {
                            let frames_available =
                                chunk.valid_frames.saturating_sub(chunk_frame_pos);
                            let frames_needed = frames_to_render - frames_written;

                            if frames_available == 0 {
                                let c = current_chunk.take().unwrap();
                                let _ = free_tx.push(c);
                                continue;
                            }

                            let to_copy = std::cmp::min(frames_available, frames_needed);

                            let src_ch = (chunk.spatial_channels as usize).max(1);
                            let target_upmix =
                                upmix_cfg.is_active_for(src_ch as i32, hardware_channels as i32);
                            if upmixer_dsp.is_blending_or_active(target_upmix) {
                                let active_ch = hardware_channels.min(12);
                                for i in 0..to_copy {
                                    let src_frame_idx = chunk_frame_pos + i;
                                    let dst_frame_idx = frames_written + i;
                                    let dst_byte_base = dst_frame_idx * frame_bytes;
                                    let fade_gain = if startup_fade_pos < startup_fade_frames {
                                        let t =
                                            startup_fade_pos as f32 / startup_fade_frames as f32;
                                        startup_fade_pos += 1;
                                        0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                    } else {
                                        1.0
                                    };
                                    let frame_slice = &chunk.samples[src_frame_idx
                                        * hardware_channels
                                        ..(src_frame_idx + 1) * hardware_channels];
                                    let out_channels = upmixer_dsp
                                        .process_multichannel_frame_for_hardware_blended(
                                            frame_slice,
                                            src_ch,
                                            &upmix_cfg,
                                            hardware_channels,
                                            target_upmix,
                                        );

                                    // Map upmixer output channels:
                                    //   out_channels = [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR, 8:TFL, 9:TFR, 10:TRL, 11:TRR]
                                    // to physical WAVEFORMATEXTENSIBLE channel bit-order:
                                    //   for 0x63F (7.1) / 0x2D63F (7.1.4): [0:FL, 1:FR, 2:FC, 3:LFE, 4:BL(RL), 5:BR(RR), 6:SL, 7:SR, ...].
                                    for c in 0..hardware_channels {
                                        let raw_s = if (src_ch < 8 || hardware_channels > 8)
                                            && hardware_channels >= 8
                                            && (negotiated_mask & 0x630) == 0x630
                                        {
                                            match c {
                                                0..=3 => out_channels[c],
                                                4 => out_channels[6],
                                                5 => out_channels[7],
                                                6 => out_channels[4],
                                                7 => out_channels[5],
                                                8..=11 => out_channels[c],
                                                _ => 0.0,
                                            }
                                        } else if c < 12 {
                                            out_channels[c]
                                        } else {
                                            0.0
                                        };
                                        let out_s = raw_s * fade_gain;
                                        write_wasapi_sample(
                                            &mut out_bytes[dst_byte_base + c * bytes_per_sample
                                                ..dst_byte_base + (c + 1) * bytes_per_sample],
                                            out_s.clamp(-1.0, 1.0),
                                            negotiated_fmt,
                                        );
                                    }

                                    for c in 0..active_ch {
                                        let s = out_channels[c] * fade_gain;
                                        upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                        channel_fft_buffers[c][fft_index] = s;
                                    }

                                    let mono =
                                        compute_fft_mono_sample(frame_slice, src_ch) * fade_gain;
                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;
                                }
                            } else {
                                for i in 0..to_copy {
                                    let src_frame_idx = chunk_frame_pos + i;
                                    let dst_frame_idx = frames_written + i;
                                    let dst_byte_base = dst_frame_idx * frame_bytes;
                                    let fade_gain = if startup_fade_pos < startup_fade_frames {
                                        let t =
                                            startup_fade_pos as f32 / startup_fade_frames as f32;
                                        startup_fade_pos += 1;
                                        0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                    } else {
                                        1.0
                                    };

                                    let mut mono = 0.0f32;
                                    for c in 0..hardware_channels {
                                        let sample = (chunk.samples
                                            [src_frame_idx * hardware_channels + c]
                                            * fade_gain)
                                            .clamp(-1.0, 1.0);
                                        write_wasapi_sample(
                                            &mut out_bytes[dst_byte_base + c * bytes_per_sample
                                                ..dst_byte_base + (c + 1) * bytes_per_sample],
                                            sample,
                                            negotiated_fmt,
                                        );
                                        let ch_weight = match c {
                                            0 | 1 => {
                                                if hardware_channels == 1 {
                                                    1.0
                                                } else {
                                                    0.5
                                                }
                                            }
                                            2 => std::f32::consts::FRAC_1_SQRT_2,
                                            _ => 0.25,
                                        };
                                        mono += sample * ch_weight;
                                        channel_fft_buffers[c][fft_index] = sample;
                                    }

                                    fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                    fft_index = (fft_index + 1) % window_size;
                                }
                            }

                            chunk_frame_pos += to_copy;
                            frames_written += to_copy;
                        }
                    }
                }

                unsafe {
                    if let Err(e) = render_client.ReleaseBuffer(avail_frames, 0) {
                        consecutive_render_errors += 1;
                        if is_fatal_device_error(e.code().0) || consecutive_render_errors >= 8 {
                            fatal_err_msg =
                                Some(format!("[audio] WASAPI ReleaseBuffer failed: {:?}", e));
                            rt_control.signal_audio_device_lost();
                            break;
                        }
                        continue;
                    }
                    consecutive_render_errors = 0;
                    if !stream_started {
                        if let Err(e) = audio_client.Start() {
                            fatal_err_msg = Some(format!("[audio] WASAPI Start() failed: {:?}", e));
                            rt_control.signal_audio_device_lost();
                            break;
                        }
                        stream_started = true;
                    }
                }
            }

            frames_since_last_dsp_send += frames_written;
            if frames_since_last_dsp_send >= dsp_send_interval
                && frames_written > 0
                && last_seek_epoch >= latest_seek_epoch
            {
                frames_since_last_dsp_send = 0;
                let play_time =
                    last_current_seconds + (chunk_frame_pos as f64 / negotiated_rate as f64);
                let (dsp_channels, vu_slice): (usize, &[f32]) = if upmix_cfg
                    .is_active_for(last_spatial_channels as i32, hardware_channels as i32)
                {
                    let active_ch = hardware_channels.min(12);
                    (active_ch, &upmix_peak_vus[..active_ch])
                } else {
                    (
                        hardware_channels.min(12).min(last_channel_vus.len().max(1)),
                        &last_channel_vus,
                    )
                };

                let msg = build_recycled_dsp_message(
                    &recycle_rx,
                    &fft_buffer,
                    &channel_fft_buffers,
                    fft_index,
                    window_size,
                    dsp_channels,
                    vu_slice,
                    last_current_order,
                    last_current_row,
                    last_bpm,
                    last_speed,
                    play_time,
                    &last_current_row_string,
                    last_seek_epoch,
                    true,
                );
                if upmix_active {
                    upmix_peak_vus = [0.0f32; 12];
                }
                let _ = tx.try_send(msg);
            }
        }

        if let Some(ref mut spatial) = spatial_state {
            unsafe {
                let _ = spatial.stream.Stop();
            }
            spatial.objects.clear();
            unsafe {
                let _ = spatial.stream.Reset();
            }
        }
        if let Some((ref audio_client, _)) = wasapi_clients {
            unsafe {
                let _ = audio_client.Stop();
                let _ = audio_client.Reset();
            }
        }
        if let Some(msg) = fatal_err_msg {
            eprintln!("{}", msg);
        }
        drop(spatial_state);
        drop(wasapi_clients);
        event.clear();
        drop(active_device);
        drop(enumerator);
    });

    #[inline(always)]
    fn write_wasapi_sample(dst: &mut [u8], sample: f32, fmt: WasapiPcmFormat) {
        match fmt {
            WasapiPcmFormat::I24In32 => {
                // 24-bit signed PCM left-aligned in 32-bit container (MSB at bit 31, LSB at bit 8)
                let v = ((sample * 8_388_607.0).round() as i32).clamp(-8_388_608, 8_388_607) << 8;
                dst.copy_from_slice(&v.to_le_bytes());
            }
            WasapiPcmFormat::F32 => {
                dst.copy_from_slice(&sample.to_le_bytes());
            }
            WasapiPcmFormat::I32 => {
                let v = (sample as f64 * 2_147_483_647.0).round() as i32;
                dst.copy_from_slice(&v.to_le_bytes());
            }
            WasapiPcmFormat::I16 => {
                let v = (sample * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16;
                dst.copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    let (
        negotiated_channels,
        negotiated_rate,
        _negotiated_fmt,
        _matched_dev_name,
        is_mpegh_bitstream,
        mpegh_carrier_label,
    ) = match init_rx.recv() {
        Ok(Ok(info)) => info,
        Ok(Err(e)) => {
            let _ = thread_handle.join();
            return Err(e);
        }
        Err(e) => {
            let _ = thread_handle.join();
            return Err(anyhow::anyhow!(
                "WASAPI Exclusive init channel closed unexpectedly: {}",
                e
            ));
        }
    };

    let mut audio_source = audio_source_opt
        .take()
        .ok_or_else(|| anyhow::anyhow!("No audio source available"))?;

    let max_frequency = {
        let mut state = shared_state.lock().unwrap();
        populate_state_from_audio_source(
            &mut state,
            audio_source.as_mut(),
            negotiated_channels as i32,
            negotiated_rate,
        );
        state.stats.mpegh_bitstream_active = is_mpegh_bitstream;
        state.stats.mpegh_carrier_info = mpegh_carrier_label;
        state.max_frequency
    };

    let (tx, rx) = bounded::<DspMessage>(32);
    let window_size = calculate_power_of_two_window_size(negotiated_rate);
    spawn_dsp_thread(
        rx,
        shared_state.clone(),
        negotiated_rate,
        max_frequency,
        window_size,
    );

    let hardware_channels = negotiated_channels as usize;
    let chunk_frames = 1024;
    let pool_size = ((negotiated_rate as usize * 8) / chunk_frames).clamp(384, 1024);

    let (ready_tx, ready_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size);
    let (mut free_tx, free_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size + 16);
    seed_audio_chunk_pool(&mut free_tx, pool_size, chunk_frames, hardware_channels);
    let (shutdown_tx, shutdown_rx) = bounded::<()>(1);

    spawn_media_decoder_threads(
        audio_source,
        shared_state.clone(),
        stop_token.clone(),
        ready_tx,
        free_rx,
        hardware_channels,
        negotiated_rate,
        chunk_frames,
        pool_size,
        4,
        file_path,
        Some(shutdown_rx),
    );

    if pipe_tx.send((ready_rx, free_tx, tx, shutdown_tx)).is_err() {
        stop_token.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = thread_handle.join();
        return Err(anyhow::anyhow!(
            "WASAPI Exclusive render thread exited before receiving audio pipes"
        ));
    }

    Ok(PlaybackHandle::Dummy(Some(thread_handle), stop_token))
}

#[inline(always)]
pub(in crate::audio) fn copy_ring_buffer(dst: &mut [f32], src: &[f32], split_idx: usize) {
    let window_size = dst.len();
    let split = split_idx.min(window_size);
    let tail_len = window_size - split;
    dst[..tail_len].copy_from_slice(&src[split..window_size]);
    dst[tail_len..window_size].copy_from_slice(&src[..split]);
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn build_recycled_dsp_message(
    recycle_rx: &Receiver<DspMessage>,
    fft_buffer: &[f32],
    channel_fft_buffers: &[Vec<f32>],
    fft_index: usize,
    window_size: usize,
    hardware_channels: usize,
    channel_vus: &[f32],
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
    current_row_string: &str,
    seek_epoch: u64,
    copy_audio: bool,
) -> DspMessage {
    let mut msg = recycle_rx.try_recv().unwrap_or_else(|_| DspMessage {
        audio_data: vec![0.0; window_size],
        channel_vus: Vec::with_capacity(hardware_channels.max(16)),
        current_order: 0,
        current_row: 0,
        bpm: 0,
        speed: 0,
        current_seconds: 0.0,
        current_row_string: String::with_capacity(256),
        channel_audio_data: vec![vec![0.0; window_size]; hardware_channels],
        seek_epoch: 0,
    });

    if copy_audio {
        if msg.audio_data.len() != window_size {
            msg.audio_data.resize(window_size, 0.0);
        }
        copy_ring_buffer(&mut msg.audio_data, fft_buffer, fft_index);

        if msg.channel_audio_data.len() != hardware_channels {
            msg.channel_audio_data
                .resize_with(hardware_channels, || vec![0.0; window_size]);
        }
        for (dst_ch, src_ch) in msg
            .channel_audio_data
            .iter_mut()
            .zip(channel_fft_buffers.iter())
            .take(hardware_channels)
        {
            if dst_ch.len() != window_size {
                dst_ch.resize(window_size, 0.0);
            }
            copy_ring_buffer(dst_ch, src_ch, fft_index);
        }
    }

    msg.channel_vus.clear();
    msg.channel_vus.extend_from_slice(channel_vus);
    msg.current_order = current_order;
    msg.current_row = current_row;
    msg.bpm = bpm;
    msg.speed = speed;
    msg.current_seconds = current_seconds;
    msg.current_row_string.clear();
    msg.current_row_string.push_str(current_row_string);
    msg.seek_epoch = seek_epoch;
    msg
}

const RT_DSP_POOL_CAPACITY: usize = 40;
const RT_MAX_VUS_CAPACITY: usize = 144;
const RT_MAX_ROW_STRING_BYTES: usize = 4096;

pub(in crate::audio) fn allocate_rt_dsp_pool(
    window_size: usize,
    max_dsp_channels: usize,
) -> (Sender<DspMessage>, Receiver<DspMessage>) {
    let (tx, rx) = bounded::<DspMessage>(RT_DSP_POOL_CAPACITY);
    for _ in 0..RT_DSP_POOL_CAPACITY {
        let _ = tx.try_send(DspMessage {
            audio_data: vec![0.0; window_size],
            channel_vus: Vec::with_capacity(RT_MAX_VUS_CAPACITY),
            current_order: 0,
            current_row: 0,
            bpm: 0,
            speed: 0,
            current_seconds: 0.0,
            current_row_string: String::with_capacity(RT_MAX_ROW_STRING_BYTES),
            channel_audio_data: (0..max_dsp_channels)
                .map(|_| vec![0.0; window_size])
                .collect(),
            seek_epoch: 0,
        });
    }
    (tx, rx)
}

#[inline]
fn copy_slice_within_capacity(dst: &mut Vec<f32>, src: &[f32]) {
    dst.clear();
    let cap = dst.capacity();
    let n = src.len().min(cap);
    dst.extend_from_slice(&src[..n]);
}

#[inline]
fn copy_str_within_capacity(dst: &mut String, src: &str) {
    dst.clear();
    let cap = dst.capacity();
    let n = src.floor_char_boundary(src.len().min(cap));
    dst.push_str(&src[..n]);
}

#[allow(clippy::too_many_arguments)]
#[inline]
pub(in crate::audio) fn try_build_rt_dsp_message(
    recycle_rx: &Receiver<DspMessage>,
    fft_buffer: &[f32],
    channel_fft_buffers: &[Vec<f32>],
    fft_index: usize,
    window_size: usize,
    active_dsp_channels: usize,
    channel_vus: &[f32],
    current_order: i32,
    current_row: i32,
    bpm: i32,
    speed: i32,
    current_seconds: f64,
    current_row_string: &str,
    seek_epoch: u64,
) -> Option<DspMessage> {
    let mut msg = recycle_rx.try_recv().ok()?;
    if msg.audio_data.capacity() >= window_size {
        if msg.audio_data.len() != window_size {
            msg.audio_data.resize(window_size, 0.0);
        }
        copy_ring_buffer(&mut msg.audio_data, fft_buffer, fft_index);
    }

    let copy_ch = active_dsp_channels
        .min(msg.channel_audio_data.len())
        .min(channel_fft_buffers.len());
    for (idx, dst_ch) in msg.channel_audio_data.iter_mut().enumerate() {
        if idx < copy_ch {
            if dst_ch.capacity() >= window_size {
                if dst_ch.len() != window_size {
                    dst_ch.resize(window_size, 0.0);
                }
                copy_ring_buffer(dst_ch, &channel_fft_buffers[idx], fft_index);
            }
        } else {
            dst_ch.clear();
        }
    }

    copy_slice_within_capacity(&mut msg.channel_vus, channel_vus);
    msg.current_order = current_order;
    msg.current_row = current_row;
    msg.bpm = bpm;
    msg.speed = speed;
    msg.current_seconds = current_seconds;
    copy_str_within_capacity(&mut msg.current_row_string, current_row_string);
    msg.seek_epoch = seek_epoch;
    Some(msg)
}

pub(in crate::audio) struct StackLowercaseAsciiBuf {
    buf: [u8; 256],
    pub(in crate::audio) len: usize,
}

impl StackLowercaseAsciiBuf {
    #[inline]
    pub(in crate::audio) fn new() -> Self {
        Self {
            buf: [0u8; 256],
            len: 0,
        }
    }

    #[inline]
    pub(in crate::audio) fn contains_subslice(&self, needle: &[u8]) -> bool {
        !needle.is_empty()
            && self.len >= needle.len()
            && self.buf[..self.len]
                .windows(needle.len())
                .any(|w| w == needle)
    }
}

impl std::fmt::Write for StackLowercaseAsciiBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let avail = self.buf.len().saturating_sub(self.len);
        let take = s.len().min(avail);
        for (dst, &b) in self.buf[self.len..self.len + take]
            .iter_mut()
            .zip(s.as_bytes().iter())
        {
            *dst = b.to_ascii_lowercase();
        }
        self.len += take;
        Ok(())
    }
}

#[inline]
fn is_recoverable_cpal_xrun(err: &cpal::Error) -> bool {
    use std::fmt::Write;
    let mut stack_buf = StackLowercaseAsciiBuf::new();
    let _ = write!(&mut stack_buf, "{}", err);
    stack_buf.contains_subslice(b"underrun") || stack_buf.contains_subslice(b"overrun")
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    audio_source_opt: &mut Option<Box<dyn AudioSource>>,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    dsp_rx_guard: Receiver<DspMessage>,
    sample_rate: u32,
    _max_frequency: f32,
    window_size: usize,
    current_path: &str,
) -> Result<(
    cpal::Stream,
    Arc<std::sync::atomic::AtomicBool>,
    Sender<DspMessage>,
    CpalStreamKeepAlive,
)>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample,
{
    let stop_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hardware_channels = config.channels as usize;

    let chunk_frames = 1024;
    let pool_size = ((sample_rate as usize * 8) / chunk_frames).clamp(384, 1024);

    let (ready_tx, ready_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size);
    let (mut free_tx, free_rx) = rtrb::RingBuffer::<AudioChunk>::new(pool_size + 16);
    seed_audio_chunk_pool(&mut free_tx, pool_size, chunk_frames, hardware_channels);
    let (shutdown_tx, shutdown_rx) = bounded::<()>(1);

    let max_dsp_channels = hardware_channels.clamp(12, 32);
    let (stream_recycle_tx, stream_recycle_rx) =
        allocate_rt_dsp_pool(window_size, max_dsp_channels);
    let keep_alive = CpalStreamKeepAlive {
        _audio_shutdown_tx: Some(shutdown_tx),
        _dsp_rx_guard: Some(dsp_rx_guard),
        _dsp_recycle_rx_guard: Some(stream_recycle_rx.clone()),
    };

    let mut current_chunk: Option<AudioChunk> = None;
    let mut chunk_frame_pos = 0;

    let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
    let mut channel_fft_buffers: Vec<Vec<f32>> =
        vec![vec![0.0; window_size]; hardware_channels.max(12)];
    let mut fft_index = 0;
    let dsp_send_interval = (sample_rate as usize / 120).max(1);
    let mut frames_since_last_dsp_send = dsp_send_interval;

    let mut last_channel_vus: Vec<f32> = Vec::with_capacity(RT_MAX_VUS_CAPACITY);
    let mut last_spatial_channels = 2usize;
    let mut last_current_order = 0;
    let mut last_current_row = 0;
    let mut last_bpm = 0;
    let mut last_speed = 0;
    let mut last_current_seconds = 0.0;
    let mut last_current_row_string: String = String::with_capacity(RT_MAX_ROW_STRING_BYTES);
    let mut last_seek_epoch = 0u64;
    let initial_src_ch = audio_source_opt
        .as_mut()
        .map(|s| s.get_num_channels())
        .unwrap_or(2);
    let (mut latest_seek_epoch, rt_control) = {
        let mut state = shared_state.lock().unwrap();
        if hardware_channels <= 2 {
            state.surround_upmix.enabled = false;
        } else if !state.surround_upmix.user_overridden {
            state.surround_upmix.enabled = !state.force_stereo_downmix
                && !state.passthrough_enabled
                && crate::state::SurroundUpmixConfig::should_enable_by_default(
                    initial_src_ch,
                    hardware_channels as i32,
                );
        }
        state.sync_rt_control();
        (state.seek_epoch, Arc::clone(&state.rt_control))
    };
    let mut upmixer_dsp = SurroundUpmixerDsp::new(sample_rate);
    let mut upmix_peak_vus = [0.0f32; 12];
    let startup_fade_frames = ((sample_rate as usize) * 35 / 1000).max(64);
    let mut startup_fade_pos = 0usize;

    let rt_control_cb = Arc::clone(&rt_control);
    let rt_control_err = rt_control;
    let mut ready_rx_cb = ready_rx;
    let mut free_tx_cb = free_tx;
    let tx_cb = tx;
    let stream_recycle_rx_cb = stream_recycle_rx;
    let stream_recycle_tx_cb = stream_recycle_tx.clone();

    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let was_paused = rt_control_cb.is_paused();
            let ctrl_epoch = rt_control_cb.seek_epoch();
            if ctrl_epoch > latest_seek_epoch {
                latest_seek_epoch = ctrl_epoch;
            }
            let need_all_channels =
                rt_control_cb.need_gpu_spectrum() || rt_control_cb.need_multichannel_raw();
            let mut upmix_cfg = rt_control_cb.load_upmix_config();
            if hardware_channels <= 2 {
                upmix_cfg.enabled = false;
            }

            if let Some(ref c) = current_chunk
                && c.seek_epoch < latest_seek_epoch
            {
                let stale = current_chunk.take().unwrap();
                let _ = free_tx_cb.push(stale);
                chunk_frame_pos = 0;
            }
            flush_stale_ready_chunks(&mut ready_rx_cb, &mut free_tx_cb, latest_seek_epoch);

            if was_paused {
                for sample in data.iter_mut() {
                    *sample = T::from_sample(0.0);
                }
                return;
            }

            let upmix_active = upmix_cfg.is_active_for_hardware(hardware_channels as i32);
            if upmix_active {
                upmixer_dsp.sync_config(&upmix_cfg, sample_rate);
            }

            let frames_to_render = data.len() / hardware_channels;
            let mut frames_written = 0;
            let mut upmix_block_buf = [[0.0f32; 12]; 128];

            while frames_written < frames_to_render {
                if current_chunk.is_none() {
                    match ready_rx_cb.pop() {
                        Ok(chunk) => {
                            if chunk.seek_epoch < latest_seek_epoch {
                                let _ = free_tx_cb.push(chunk);
                                continue;
                            }
                            copy_slice_within_capacity(&mut last_channel_vus, &chunk.channel_vus);
                            last_spatial_channels = (chunk.spatial_channels as usize).max(1);
                            last_current_order = chunk.current_order;
                            last_current_row = chunk.current_row;
                            last_bpm = chunk.bpm;
                            last_speed = chunk.speed;
                            last_current_seconds = chunk.current_seconds;
                            copy_str_within_capacity(
                                &mut last_current_row_string,
                                &chunk.current_row_string,
                            );
                            last_seek_epoch = chunk.seek_epoch;
                            if chunk.seek_epoch > latest_seek_epoch {
                                latest_seek_epoch = chunk.seek_epoch;
                            }

                            if chunk.track_ended {
                                rt_control_cb.signal_track_ended();
                            }

                            current_chunk = Some(chunk);
                            chunk_frame_pos = 0;
                        }
                        Err(_) => {
                            for frame in data[frames_written * hardware_channels..]
                                .chunks_mut(hardware_channels)
                            {
                                for sample in frame.iter_mut() {
                                    *sample = T::from_sample(0.0);
                                }
                            }
                            break;
                        }
                    }
                }

                if let Some(chunk) = current_chunk.as_mut() {
                    let frames_available = chunk.valid_frames.saturating_sub(chunk_frame_pos);
                    let frames_needed = frames_to_render - frames_written;

                    if frames_available == 0 {
                        let c = current_chunk.take().unwrap();
                        let _ = free_tx_cb.push(c);
                        continue;
                    }

                    let to_copy = std::cmp::min(frames_available, frames_needed);

                    let src_ch = (chunk.spatial_channels as usize).max(1);
                    let target_upmix =
                        upmix_cfg.is_active_for(src_ch as i32, hardware_channels as i32);
                    if src_ch <= 2
                        && hardware_channels <= 12
                        && upmixer_dsp.is_fully_active(target_upmix)
                    {
                        let active_ch = hardware_channels;
                        let mut offset = 0usize;
                        while offset < to_copy {
                            let batch = (to_copy - offset).min(upmix_block_buf.len());
                            let in_start = (chunk_frame_pos + offset) * hardware_channels;
                            let in_end = in_start + batch * hardware_channels;
                            upmixer_dsp.process_interleaved_stereo_block(
                                &chunk.samples[in_start..in_end],
                                hardware_channels,
                                src_ch,
                                &mut upmix_block_buf[..batch],
                                &upmix_cfg,
                                hardware_channels,
                            );
                            for (i, out_channels) in upmix_block_buf.iter().enumerate().take(batch)
                            {
                                let src_frame_idx = chunk_frame_pos + offset + i;
                                let dst_frame_idx = frames_written + offset + i;
                                let fade_gain = if startup_fade_pos < startup_fade_frames {
                                    let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                    startup_fade_pos += 1;
                                    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                                } else {
                                    1.0
                                };
                                for (c, slot) in data[dst_frame_idx * hardware_channels
                                    ..(dst_frame_idx + 1) * hardware_channels]
                                    .iter_mut()
                                    .enumerate()
                                {
                                    let raw_s = if hardware_channels == 7 {
                                        match c {
                                            4 => out_channels[6],
                                            5 => out_channels[4],
                                            6 => out_channels[5],
                                            _ => out_channels[c],
                                        }
                                    } else if hardware_channels >= 8 {
                                        match c {
                                            0..=3 => out_channels[c],
                                            4 => out_channels[6],
                                            5 => out_channels[7],
                                            6 => out_channels[4],
                                            7 => out_channels[5],
                                            8..=11 => out_channels[c],
                                            _ => 0.0,
                                        }
                                    } else {
                                        out_channels[c]
                                    };
                                    let out_s = raw_s * fade_gain;
                                    *slot = T::from_sample(out_s.clamp(-1.0, 1.0));
                                }

                                for c in 0..active_ch {
                                    let s = out_channels[c] * fade_gain;
                                    upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                    if need_all_channels || c == 0 {
                                        channel_fft_buffers[c][fft_index] = s;
                                    }
                                }

                                let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                    ..(src_frame_idx + 1) * hardware_channels];
                                let mono = compute_fft_mono_sample(frame_slice, src_ch) * fade_gain;
                                fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                                fft_index = (fft_index + 1) % window_size;
                            }
                            offset += batch;
                        }
                    } else if upmixer_dsp.is_blending_or_active(target_upmix) {
                        let active_ch = hardware_channels.min(12);
                        for i in 0..to_copy {
                            let src_frame_idx = chunk_frame_pos + i;
                            let dst_frame_idx = frames_written + i;
                            let fade_gain = if startup_fade_pos < startup_fade_frames {
                                let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                startup_fade_pos += 1;
                                0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                            } else {
                                1.0
                            };
                            let frame_slice = &chunk.samples[src_frame_idx * hardware_channels
                                ..(src_frame_idx + 1) * hardware_channels];
                            let out_channels = upmixer_dsp
                                .process_multichannel_frame_for_hardware_blended(
                                    frame_slice,
                                    src_ch,
                                    &upmix_cfg,
                                    hardware_channels,
                                    target_upmix,
                                );

                            for c in 0..hardware_channels {
                                let raw_s = if hardware_channels == 7 {
                                    // Map canonical 6.1 [0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:BC]
                                    // to physical WAVEFORMATEXTENSIBLE / ALSA 6.1 order
                                    // [0:FL, 1:FR, 2:FC, 3:LFE, 4:BC, 5:SL, 6:SR].
                                    match c {
                                        4 => out_channels[6],
                                        5 => out_channels[4],
                                        6 => out_channels[5],
                                        _ => out_channels[c],
                                    }
                                } else if (src_ch < 8 || hardware_channels > 8)
                                    && hardware_channels >= 8
                                {
                                    // Map canonical 7.1/7.1.x [4:SL, 5:SR, 6:RL, 7:RR] to
                                    // physical [4:BL, 5:BR, 6:SL, 7:SR].
                                    match c {
                                        0..=3 => out_channels[c],
                                        4 => out_channels[6],
                                        5 => out_channels[7],
                                        6 => out_channels[4],
                                        7 => out_channels[5],
                                        8..=11 => out_channels[c],
                                        _ => 0.0,
                                    }
                                } else if c < 12 {
                                    out_channels[c]
                                } else {
                                    0.0
                                };
                                let out_s = raw_s * fade_gain;
                                data[dst_frame_idx * hardware_channels + c] =
                                    T::from_sample(out_s.clamp(-1.0, 1.0));
                            }

                            for c in 0..active_ch {
                                let s = out_channels[c] * fade_gain;
                                upmix_peak_vus[c] = upmix_peak_vus[c].max(s.abs());
                                if need_all_channels || c == 0 {
                                    channel_fft_buffers[c][fft_index] = s;
                                }
                            }

                            let mono = compute_fft_mono_sample(frame_slice, src_ch) * fade_gain;
                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    } else {
                        for i in 0..to_copy {
                            let src_frame_idx = chunk_frame_pos + i;
                            let dst_frame_idx = frames_written + i;
                            let fade_gain = if startup_fade_pos < startup_fade_frames {
                                let t = startup_fade_pos as f32 / startup_fade_frames as f32;
                                startup_fade_pos += 1;
                                0.5 - 0.5 * (std::f32::consts::PI * t).cos()
                            } else {
                                1.0
                            };

                            let mut mono = 0.0;
                            for c in 0..hardware_channels {
                                let sample = (chunk.samples[src_frame_idx * hardware_channels + c]
                                    * fade_gain)
                                    .clamp(-1.0, 1.0);
                                data[dst_frame_idx * hardware_channels + c] =
                                    T::from_sample(sample);
                                let ch_weight = match c {
                                    0 | 1 => {
                                        if hardware_channels == 1 {
                                            1.0
                                        } else {
                                            0.5
                                        }
                                    }
                                    2 => std::f32::consts::FRAC_1_SQRT_2,
                                    _ => 0.25,
                                };
                                mono += sample * ch_weight;
                                if need_all_channels || c == 0 {
                                    channel_fft_buffers[c][fft_index] = sample;
                                }
                            }

                            fft_buffer[fft_index] = mono.clamp(-1.0, 1.0);
                            fft_index = (fft_index + 1) % window_size;
                        }
                    }

                    chunk_frame_pos += to_copy;
                    frames_written += to_copy;
                }
            }

            frames_since_last_dsp_send += frames_written;
            if frames_since_last_dsp_send >= dsp_send_interval
                && frames_written > 0
                && last_seek_epoch >= latest_seek_epoch
            {
                frames_since_last_dsp_send = 0;
                let play_time =
                    last_current_seconds + (chunk_frame_pos as f64 / sample_rate as f64);
                let (dsp_channels, vu_slice): (usize, &[f32]) = if upmix_cfg
                    .is_active_for(last_spatial_channels as i32, hardware_channels as i32)
                {
                    let active_ch = hardware_channels.min(12);
                    (active_ch, &upmix_peak_vus[..active_ch])
                } else {
                    (
                        hardware_channels.min(32).min(last_channel_vus.len().max(1)),
                        &last_channel_vus,
                    )
                };
                if let Some(msg) = try_build_rt_dsp_message(
                    &stream_recycle_rx_cb,
                    &fft_buffer,
                    &channel_fft_buffers,
                    fft_index,
                    window_size,
                    if need_all_channels {
                        dsp_channels
                    } else {
                        dsp_channels.min(1)
                    },
                    vu_slice,
                    last_current_order,
                    last_current_row,
                    last_bpm,
                    last_speed,
                    play_time,
                    &last_current_row_string,
                    last_seek_epoch,
                ) && let Err(crossbeam_channel::TrySendError::Full(unsent)) = tx_cb.try_send(msg)
                {
                    let _ = stream_recycle_tx_cb.try_send(unsent);
                }
                if upmix_active {
                    upmix_peak_vus = [0.0f32; 12];
                }
            }
        },
        move |err| match err.kind() {
            cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
                rt_control_err.signal_audio_device_lost();
            }
            cpal::ErrorKind::Xrun
            | cpal::ErrorKind::DeviceChanged
            | cpal::ErrorKind::RealtimeDenied => {}
            _ => {
                if !is_recoverable_cpal_xrun(&err) {
                    rt_control_err.signal_audio_device_lost();
                }
            }
        },
        None,
    )?;

    // Populate state and pre-buffer initial prime chunks BEFORE calling stream.play()
    // so the real-time audio callback never underruns or misses the initial upmix config lock.
    let mut audio_source = audio_source_opt
        .take()
        .ok_or_else(|| anyhow::anyhow!("No audio source available"))?;
    {
        let mut state = shared_state.lock().unwrap();
        populate_state_from_audio_source(
            &mut state,
            audio_source.as_mut(),
            config.channels as i32,
            config.sample_rate,
        );
    }

    spawn_media_decoder_threads(
        audio_source,
        shared_state,
        stop_token.clone(),
        ready_tx,
        free_rx,
        hardware_channels,
        sample_rate,
        chunk_frames,
        pool_size,
        4,
        current_path,
        Some(shutdown_rx),
    );

    if let Err(e) = stream.play().context("Failed to play stream") {
        stop_token.store(true, std::sync::atomic::Ordering::Relaxed);
        return Err(e);
    }

    Ok((stream, stop_token, stream_recycle_tx, keep_alive))
}

fn run_mic<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared_state: Arc<Mutex<AppState>>,
    tx: Sender<DspMessage>,
    dsp_rx_guard: Receiver<DspMessage>,
    _sample_rate: u32,
) -> Result<(cpal::Stream, Sender<DspMessage>, CpalStreamKeepAlive)>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample + Into<f32>,
{
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.max(1);
    let window_size = calculate_power_of_two_window_size(config.sample_rate);

    let max_dsp_channels = channels.clamp(1, 32);
    let (stream_recycle_tx, stream_recycle_rx) =
        allocate_rt_dsp_pool(window_size, max_dsp_channels);
    let keep_alive = CpalStreamKeepAlive {
        _audio_shutdown_tx: None,
        _dsp_rx_guard: Some(dsp_rx_guard),
        _dsp_recycle_rx_guard: Some(stream_recycle_rx.clone()),
    };

    let mut fft_buffer: Vec<f32> = vec![0.0; window_size];
    let mut channel_fft_buffers: Vec<Vec<f32>> = vec![vec![0.0; window_size]; channels];
    let mut channel_peaks: Vec<f32> = vec![0.0; channels.max(1)];
    let mut fft_index = 0;
    let dsp_send_interval = (config.sample_rate as usize / 120).max(1);
    let mut frames_since_last_dsp_send = dsp_send_interval;
    let mut captured_frames: u64 = 0;

    let rt_control = {
        let mut state = shared_state.lock().unwrap();
        state.sync_rt_control();
        Arc::clone(&state.rt_control)
    };
    let rt_control_cb = Arc::clone(&rt_control);
    let rt_control_err = rt_control;
    let stream_recycle_rx_cb = stream_recycle_rx;
    let stream_recycle_tx_cb = stream_recycle_tx.clone();

    let stream = device.build_input_stream(
        *config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if rt_control_cb.is_paused() {
                return;
            }
            let latest_seek_epoch = rt_control_cb.seek_epoch();
            let need_all_channels =
                rt_control_cb.need_gpu_spectrum() || rt_control_cb.need_multichannel_raw();

            channel_peaks.fill(0.0);
            let mut frames_written = 0;

            for frame in data.chunks(channels.max(1)) {
                let left = frame[0].into();
                let right = if channels >= 2 { frame[1].into() } else { left };

                let mono = (left + right) / 2.0;
                fft_buffer[fft_index] = mono;
                for c in 0..channels {
                    let s: f32 = if c < frame.len() {
                        frame[c].into()
                    } else {
                        0.0
                    };
                    channel_peaks[c] = channel_peaks[c].max(s.abs());
                    if need_all_channels || c == 0 {
                        channel_fft_buffers[c][fft_index] = s;
                    }
                }
                fft_index = (fft_index + 1) % window_size;
                frames_written += 1;
            }

            captured_frames = captured_frames.wrapping_add(frames_written as u64);
            frames_since_last_dsp_send += frames_written;
            if frames_since_last_dsp_send >= dsp_send_interval {
                frames_since_last_dsp_send = 0;
                let vu_slice = &channel_peaks[..channels];
                let current_secs = captured_frames as f64 / sample_rate as f64;

                if let Some(msg) = try_build_rt_dsp_message(
                    &stream_recycle_rx_cb,
                    &fft_buffer,
                    &channel_fft_buffers,
                    fft_index,
                    window_size,
                    if need_all_channels {
                        channels.min(32)
                    } else {
                        channels.min(1)
                    },
                    vu_slice,
                    0,
                    0,
                    0,
                    0,
                    current_secs,
                    "",
                    latest_seek_epoch,
                ) && let Err(crossbeam_channel::TrySendError::Full(unsent)) = tx.try_send(msg)
                {
                    let _ = stream_recycle_tx_cb.try_send(unsent);
                }
            }
        },
        move |err| match err.kind() {
            cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
                rt_control_err.signal_audio_device_lost();
            }
            cpal::ErrorKind::Xrun
            | cpal::ErrorKind::DeviceChanged
            | cpal::ErrorKind::RealtimeDenied => {}
            _ => {
                if !is_recoverable_cpal_xrun(&err) {
                    rt_control_err.signal_audio_device_lost();
                }
            }
        },
        None,
    )?;

    stream.play().context("Failed to play input stream")?;

    Ok((stream, stream_recycle_tx, keep_alive))
}
