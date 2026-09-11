#[cfg(target_os = "linux")]
pub fn start_bitstream_thread(
    file_path: &str,
    _shared_state: std::sync::Arc<std::sync::Mutex<crate::state::AppState>>,
    tx: crossbeam_channel::Sender<crate::audio::DspMessage>,
    stop_token: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<(std::thread::JoinHandle<()>, u32, u16, String, bool)> {
    use anyhow::Context;

    println!("[bitstream] Probing codec via ffmpeg-next on Linux...");
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
    ffmpeg_next::init().context("Failed to initialize ffmpeg-next")?;
    
    let mut dict = ffmpeg_next::Dictionary::new();
    dict.set("probesize", "5000000");
    dict.set("analyzeduration", "5000000");
    let mut ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
        .context("Failed to open input file")?;
        
    let best_audio = ictx.streams().best(ffmpeg_next::media::Type::Audio)
        .ok_or_else(|| anyhow::anyhow!("No audio stream found"))?;
        
    let codec_id = best_audio.parameters().id();
    let codec_name = match codec_id {
        ffmpeg_next::codec::Id::TRUEHD => "truehd",
        ffmpeg_next::codec::Id::EAC3 => "eac3",
        ffmpeg_next::codec::Id::DTS => "dts",
        ffmpeg_next::codec::Id::AC3 => "ac3",
        _ => return Err(anyhow::anyhow!("Unsupported codec for bitstreaming: {:?}", codec_id)),
    }.to_string();

    let has_video = ictx.streams().best(ffmpeg_next::media::Type::Video).is_some();

    let parameters = best_audio.parameters();
    let best_audio_index = best_audio.index();

    let probe_ctx = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone())
        .context("Failed to create probe context")?;
    let probe_decoder = probe_ctx.decoder().audio()
        .context("Failed to create probe decoder")?;
    let decoder_sample_rate = probe_decoder.rate();
    
    let pw_rate = if codec_name == "truehd" || codec_name == "dts" { 192000 } else { 48000 };
    println!("[bitstream] Codec: {}, Decoder Rate: {}, Output Rate: {}", codec_name, decoder_sample_rate, pw_rate);

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
        return Err(anyhow::anyhow!("mkfifo failed: {}", String::from_utf8_lossy(&mkfifo_out.stderr)));
    }

    let mut child = std::process::Command::new("pw-play")
        .arg("--properties=node.passthrough=true")
        .arg("-f").arg("s16")
        .arg("-r").arg(pw_rate.to_string())
        .arg("-c").arg("2")
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
        return Err(anyhow::anyhow!("pw-play exited immediately with status: {}", status));
    }

    let pipe_path_clone = pipe_path.clone();

    let ffmpeg_thread = std::thread::spawn(move || {
        println!("[bitstream] FFmpeg thread started on Linux.");
        
        let mut octx = ffmpeg_next::format::output_as(&pipe_path_clone, "spdif").unwrap();
        let ost_index = {
            let mut ost = octx.add_stream(ffmpeg_next::codec::Id::None).unwrap();
            ost.set_parameters(parameters.clone());
            ost.index()
        };
        
        let mut dict = ffmpeg_next::Dictionary::new();
        dict.set("flush_packets", "1");
        if octx.write_header_with(dict).is_err() {
            println!("[bitstream] Failed to write header to pipe");
            let _ = child.kill();
            let _ = std::fs::remove_file(&pipe_path_clone);
            return;
        }
        let ost_time_base = octx.stream(ost_index).unwrap().time_base();

        let decoder_context = match ffmpeg_next::codec::context::Context::from_parameters(parameters.clone()) {
            Ok(c) => c,
            Err(e) => {
                println!("[bitstream] Failed to create decoder context: {}", e);
                return;
            }
        };
        let mut decoder = match decoder_context.decoder().audio() {
            Ok(d) => d,
            Err(e) => {
                println!("[bitstream] Failed to create decoder: {}", e);
                return;
            }
        };
        let src_channel_layout = if decoder.channel_layout().channels() > 0 {
            decoder.channel_layout()
        } else {
            ffmpeg_next::channel_layout::ChannelLayout::default(decoder.channels().max(1) as i32)
        };
        let vis_channels = (decoder.channels() as i32).clamp(2, 8);
        let target_channel_layout = ffmpeg_next::channel_layout::ChannelLayout::default(vis_channels);
        let mut resampler = match ffmpeg_next::software::resampling::context::Context::get(
            decoder.format(), src_channel_layout, decoder.rate(),
            ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
            target_channel_layout, decoder.rate(),
        ) {
            Ok(r) => r,
            Err(e) => {
                println!("[bitstream] Failed to create resampler: {}", e);
                return;
            }
        };

        let decoder_rate = decoder.rate() as f32;
        let window_size = crate::audio::calculate_power_of_two_window_size(decoder.rate());
        let update_interval = (decoder_rate / 60.0).ceil() as usize;
        let mut accumulator: Vec<Vec<f32>> = Vec::new();
        let mut samples_since_last_send = 0;

        let mut current_seconds = 0.0;
        
        for (stream, mut packet) in ictx.packets() {
            if stop_token.load(std::sync::atomic::Ordering::Relaxed) {
                println!("[bitstream] Stop token received, stopping bitstream.");
                break;
            }
            if stream.index() == best_audio_index {
                let vis_packet = packet.clone();

                if let Some(pts) = packet.pts() {
                    current_seconds = pts as f64 * f64::from(stream.time_base());
                }

                packet.rescale_ts(stream.time_base(), ost_time_base);
                packet.set_stream(ost_index);
                if packet.write(&mut octx).is_err() {
                    break; // Pipe broken
                }

                if decoder.send_packet(&vis_packet).is_ok() {
                    let mut frame = ffmpeg_next::frame::Audio::empty();
                    while decoder.receive_frame(&mut frame).is_ok() {
                        let mut resampled = ffmpeg_next::frame::Audio::empty();
                        if resampler.run(&frame, &mut resampled).is_ok() {
                            let planes = resampled.channels() as usize;
                            if accumulator.len() != planes {
                                accumulator = vec![Vec::new(); planes];
                            }
                            
                            let mut fresh_samples = 0;
                            for (p, acc) in accumulator.iter_mut().enumerate().take(planes) {
                                let data = resampled.plane::<f32>(p);
                                if p == 0 { fresh_samples = data.len(); }
                                acc.extend_from_slice(data);
                                let excess = acc.len().saturating_sub(window_size);
                                if excess > 0 {
                                    acc.drain(0..excess);
                                }
                            }
                            
                            samples_since_last_send += fresh_samples;
                            
                            if accumulator.first().map(|a| a.len()).unwrap_or(0) == window_size && samples_since_last_send >= update_interval {
                                samples_since_last_send = 0;
                                let mut channel_audio_data = Vec::with_capacity(planes);
                                let mut channel_vus = Vec::with_capacity(planes);
                                let eval_len = fresh_samples.max(update_interval).min(window_size);
                                
                                for acc in accumulator.iter().take(planes) {
                                    let window = acc.clone();
                                    let mut peak = 0.0f32;
                                    let start_idx = window.len().saturating_sub(eval_len);
                                    for &s in &window[start_idx..] {
                                        peak = peak.max(s.abs());
                                    }
                                    channel_vus.push(peak.clamp(0.0, 1.0));
                                    channel_audio_data.push(window);
                                }

                                let mut mono_audio_data = vec![0.0f32; window_size];
                                for acc in accumulator.iter().take(planes) {
                                    for (i, &sample) in acc.iter().take(window_size).enumerate() {
                                        mono_audio_data[i] += sample;
                                    }
                                }
                                let inv_planes = 1.0 / planes as f32;
                                for s in &mut mono_audio_data {
                                    *s *= inv_planes;
                                }
                                
                                let _ = tx.try_send(crate::audio::DspMessage {
                                    audio_data: mono_audio_data,
                                    channel_vus,
                                    current_order: 0,
                                    current_row: 0,
                                    bpm: 0,
                                    speed: 0,
                                    current_seconds,
                                    current_row_string: "".to_string(),
                                    channel_audio_data,
                                });
                            }
                        }
                    }
                }
            }
        }
        let _ = octx.write_trailer();
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&pipe_path_clone);
        println!("[bitstream] FFmpeg thread finished on Linux.");
    });

    Ok((ffmpeg_thread, decoder_sample_rate as u32, 8u16, codec_name, has_video))
}

#[cfg(target_os = "macos")]
mod macos_bitstream {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};
    use crossbeam_channel::Sender;
    use crate::state::AppState;
    use crate::audio::DspMessage;
    use anyhow::{Context, Result};
    use std::ptr;
    use std::mem;

    type OSStatus = i32;
    type AudioObjectID = u32;
    type AudioDeviceID = AudioObjectID;
    type AudioObjectPropertySelector = u32;
    type AudioObjectPropertyScope = u32;
    type AudioObjectPropertyElement = u32;

    #[repr(C)]
    struct AudioObjectPropertyAddress {
        mSelector: AudioObjectPropertySelector,
        mScope: AudioObjectPropertyScope,
        mElement: AudioObjectPropertyElement,
    }

    const kAudioObjectSystemObject: AudioObjectID = 1;
    const kAudioObjectPropertyScopeGlobal: AudioObjectPropertyScope = 0x676c6f62; // 'glob'
    const kAudioObjectPropertyElementMain: AudioObjectPropertyElement = 0;

    const kAudioHardwarePropertyDefaultOutputDevice: AudioObjectPropertySelector = 0x6465666f; // 'defo'
    const kAudioDevicePropertyHogMode: AudioObjectPropertySelector = 0x6f686f67; // 'ohog'

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyData(
            inObjectID: AudioObjectID,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const std::ffi::c_void,
            ioDataSize: *mut u32,
            outData: *mut std::ffi::c_void,
        ) -> OSStatus;

        fn AudioObjectSetPropertyData(
            inObjectID: AudioObjectID,
            inAddress: *const AudioObjectPropertyAddress,
            inQualifierDataSize: u32,
            inQualifierData: *const std::ffi::c_void,
            inDataSize: u32,
            inData: *const std::ffi::c_void,
        ) -> OSStatus;
    }

    pub fn start_bitstream_thread(
        file_path: &str,
        _shared_state: Arc<Mutex<AppState>>,
        _tx: Sender<DspMessage>,
        stop_token: Arc<AtomicBool>,
    ) -> Result<(std::thread::JoinHandle<()>, u32, u16, String, bool)> {
        println!("[bitstream] Initializing macOS CoreAudio passthrough (Hog Mode)...");

        println!("[bitstream] Probing audio stream via ffmpeg-next...");
        ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
        ffmpeg_next::init().context("Failed to initialize ffmpeg-next")?;

        let mut dict = ffmpeg_next::Dictionary::new();
        dict.set("probesize", "5000000");
        dict.set("analyzeduration", "5000000");
        let mut ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
            .context("Failed to open input file")?;

        let best_audio = ictx.streams().best(ffmpeg_next::media::Type::Audio)
            .ok_or_else(|| anyhow::anyhow!("No audio stream found"))?;

        let codec_id = best_audio.parameters().id();
        let codec_name = match codec_id {
            ffmpeg_next::codec::Id::TRUEHD => "truehd",
            ffmpeg_next::codec::Id::EAC3 => "eac3",
            ffmpeg_next::codec::Id::DTS => "dts",
            ffmpeg_next::codec::Id::AC3 => "ac3",
            _ => return Err(anyhow::anyhow!("Unsupported codec for bitstreaming: {:?}", codec_id)),
        }.to_string();

        let has_video = ictx.streams().best(ffmpeg_next::media::Type::Video).is_some();
        let parameters = best_audio.parameters();

        let probe_ctx = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone())
            .context("Failed to create probe context")?;
        let probe_decoder = probe_ctx.decoder().audio()
            .context("Failed to create probe decoder")?;
        let decoder_sample_rate = probe_decoder.rate();
        println!("[bitstream] Codec: {}, Decoder Rate: {} Hz", codec_name, decoder_sample_rate);

        let mut device_id: AudioDeviceID = 0;
        let mut data_size = mem::size_of::<AudioDeviceID>() as u32;
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                0,
                ptr::null(),
                &mut data_size,
                &mut device_id as *mut _ as *mut std::ffi::c_void,
            )
        };

        if status != 0 {
            return Err(anyhow::anyhow!("Failed to query default output device. CoreAudio OSStatus: {}", status));
        }
        println!("[bitstream] Default output device ID: {}", device_id);

        println!("[bitstream] Requesting CoreAudio Hog Mode for device {}...", device_id);
        let hog_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyHogMode,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };

        let my_pid = std::process::id() as i32;
        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &hog_address,
                0,
                ptr::null(),
                mem::size_of::<i32>() as u32,
                &my_pid as *const _ as *const std::ffi::c_void,
            )
        };

        if status != 0 {
            return Err(anyhow::anyhow!(
                "Failed to acquire Hog Mode (OSStatus: {}). The device may be in use by another process.",
                status
            ));
        }

        // Release Hog Mode immediately and return clean informative error
        let release_pid: i32 = -1;
        unsafe {
            let _ = AudioObjectSetPropertyData(
                device_id,
                &hog_address,
                0,
                ptr::null(),
                mem::size_of::<i32>() as u32,
                &release_pid as *const _ as *const std::ffi::c_void,
            );
        };
        Err(anyhow::anyhow!(
            "macOS CoreAudio bitstream HAL packetization (IEC 61937) is currently under active development. Standard decoded PCM playback is recommended on macOS."
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
) -> anyhow::Result<(std::thread::JoinHandle<()>, u32, u16, String, bool)> {
    Err(anyhow::anyhow!("Bitstream passthrough is not supported on this platform."))
}
#[cfg(windows)]
pub use wasapi_bitstream::start_bitstream_thread;

#[cfg(windows)]
mod wasapi_bitstream {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};
    use crossbeam_channel::Sender;
    use crate::state::AppState;
    use crate::audio::DspMessage;
    use anyhow::{Context, Result};
    use std::io::Read;
    use std::ptr;
    use windows::core::GUID;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    const WAIT_OBJECT_0: u32 = 0;
    static PIPE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    // ─── Standard PCM & IEEE Float WASAPI SubFormat GUIDs ──────────
    const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID {
        data1: 0x00000001, data2: 0x0000, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID = GUID {
        data1: 0x00000003, data2: 0x0000, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    // ─── IEC 61937 WASAPI SubFormat GUIDs ────────────────────────────
    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL: GUID = GUID {
        data1: 0x00000092, data2: 0x0000, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS: GUID = GUID {
        data1: 0x0000000a, data2: 0x0cea, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20: GUID = GUID {
        data1: 0x00000017, data2: 0x0cea, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DTS: GUID = GUID {
        data1: 0x00000008, data2: 0x0000, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD: GUID = GUID {
        data1: 0x0000000b, data2: 0x0cea, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MLP: GUID = GUID {
        data1: 0x0000000c, data2: 0x0cea, data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    };

    const PKEY_DEVICE_FRIENDLY_NAME: windows::Win32::Foundation::PROPERTYKEY = windows::Win32::Foundation::PROPERTYKEY {
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
            let store = device.OpenPropertyStore(windows::Win32::System::Com::STGM_READ).ok()?;
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
            // Uncompressed Multi-Channel LPCM
            id if id == ffmpeg_next::codec::Id::FLAC
                || id == ffmpeg_next::codec::Id::ALAC
                || id == ffmpeg_next::codec::Id::WAVPACK
                || format!("{:?}", id).starts_with("PCM_")
                || src_channels > 2 =>
            {
                let codec_name = match id {
                    ffmpeg_next::codec::Id::FLAC => "flac",
                    ffmpeg_next::codec::Id::ALAC => "alac",
                    ffmpeg_next::codec::Id::WAVPACK => "wavpack",
                    _ if format!("{:?}", id).starts_with("PCM_") => "pcm",
                    _ => "multichannel_pcm",
                }.to_string();

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
                            name: format!("Multi-Channel LPCM 32-bit Float ({}ch x {}Hz, mask 0x{:X})", ch, rate, mask),
                            stream_type: WasapiStreamType::Lpcm,
                            channels: ch,
                            rate,
                            valid_bits: 32,
                            container_bits: 32,
                            channel_mask: mask,
                            sub_format: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
                            sample_format: ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                        });
                        profiles.push(AudioProfile {
                            name: format!("Multi-Channel LPCM 16-bit ({}ch x {}Hz, mask 0x{:X})", ch, rate, mask),
                            stream_type: WasapiStreamType::Lpcm,
                            channels: ch,
                            rate,
                            valid_bits: 16,
                            container_bits: 16,
                            channel_mask: mask,
                            sub_format: KSDATAFORMAT_SUBTYPE_PCM,
                            sample_format: ffmpeg_next::format::sample::Sample::I16(ffmpeg_next::format::sample::Type::Packed),
                        });
                        profiles.push(AudioProfile {
                            name: format!("Multi-Channel LPCM 24-bit ({}ch x {}Hz, mask 0x{:X})", ch, rate, mask),
                            stream_type: WasapiStreamType::Lpcm,
                            channels: ch,
                            rate,
                            valid_bits: 24,
                            container_bits: 32,
                            channel_mask: mask,
                            sub_format: KSDATAFORMAT_SUBTYPE_PCM,
                            sample_format: ffmpeg_next::format::sample::Sample::I32(ffmpeg_next::format::sample::Type::Packed),
                        });
                    }
                }
                Ok((codec_name, WasapiStreamType::Lpcm, profiles))
            }
            _ => Err(anyhow::anyhow!("Unsupported codec for bitstreaming or exclusive multi-channel LPCM: {:?}", codec_id)),
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
                    Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: 16 },
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
                        ).to_vec()
                    }
                } else {
                    unsafe {
                        std::slice::from_raw_parts(
                            &format_ext as *const _ as *const u8,
                            std::mem::size_of::<WAVEFORMATEXTENSIBLE>(),
                        ).to_vec()
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
                    Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: profile.valid_bits },
                    dwChannelMask: profile.channel_mask,
                    SubFormat: profile.sub_format,
                };

                unsafe {
                    std::slice::from_raw_parts(
                        &format_ext as *const _ as *const u8,
                        std::mem::size_of::<WAVEFORMATEXTENSIBLE>(),
                    ).to_vec()
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn list_devices() -> Result<()> {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }

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

        unsafe { CoUninitialize(); }
        Ok(())
    }

    pub fn start_bitstream_thread(
        file_path: &str,
        shared_state: Arc<Mutex<AppState>>,
        tx: Sender<DspMessage>,
        stop_token: Arc<AtomicBool>,
    ) -> Result<(std::thread::JoinHandle<()>, u32, u16, String, bool)> {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }

        // ── Probe codec via ffmpeg-next ─────────────────────────────
        println!("[bitstream] Probing audio stream via ffmpeg-next on Windows...");
        ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
        ffmpeg_next::init().context("Failed to initialize ffmpeg-next")?;
        
        let mut dict = ffmpeg_next::Dictionary::new();
        dict.set("probesize", "5000000");
        dict.set("analyzeduration", "5000000");
        let mut ictx = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
            .context("Failed to open input file")?;
            
        let best_audio = ictx.streams().best(ffmpeg_next::media::Type::Audio)
            .ok_or_else(|| anyhow::anyhow!("No audio stream found"))?;
            
        let codec_id = best_audio.parameters().id();
        let best_audio_index = best_audio.index();
        let parameters = best_audio.parameters();

        let probe_ctx = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone())
            .context("Failed to create probe context")?;
        let probe_decoder = probe_ctx.decoder().audio()
            .context("Failed to create probe decoder")?;
        let decoder_sample_rate = probe_decoder.rate();
        let src_channels = probe_decoder.channels();

        let (codec_name, stream_type, profiles) = detect_codec_profile(codec_id, src_channels, decoder_sample_rate)?;
        let has_video = ictx.streams().best(ffmpeg_next::media::Type::Video).is_some();

        println!("[bitstream] Detected codec: {}, Decoder Rate: {} Hz, Channels: {}", codec_name, decoder_sample_rate, src_channels);
        for p in &profiles {
            println!("  [bitstream] Candidate profile: {} ({}ch x {}Hz)", p.name, p.channels, p.rate);
        }

        // ── Open device ─────────────────────────────────────────────
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };

        let selected_device_name = shared_state.lock().ok().and_then(|s| s.selected_audio_device.clone());
        println!("[bitstream] Selected audio device in state: {:?}", selected_device_name);

        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
        let count = unsafe { collection.GetCount()? };
        
        let mut target_device: Option<IMMDevice> = None;

        // 1. If user selected a device, search for it
        if let Some(target) = &selected_device_name {
            let target_lower = target.to_lowercase();
            for i in 0..count {
                if let Ok(dev) = unsafe { collection.Item(i) } {
                    let dev_name = get_device_name(&dev).unwrap_or_default();
                    let dev_id = unsafe { dev.GetId().ok().and_then(|id| id.to_string().ok()).unwrap_or_default() };
                    if dev_name.to_lowercase().contains(&target_lower) || target_lower.contains(&dev_name.to_lowercase()) || dev_id == *target {
                        println!("[bitstream] Found matching device for '{}': {}", target, dev_name);
                        target_device = Some(dev);
                        break;
                    }
                }
            }
        }

        let default_device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok() };

        // Helper to test if a device accepts any of the given profiles
        let test_device_formats = |dev: &IMMDevice, profs: &[AudioProfile]| -> Option<(IAudioClient, AudioProfile, Vec<u8>)> {
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
                    println!("  [bitstream] Device accepted profile: {} ({}ch x {}Hz)", p.name, p.channels, p.rate);
                    return Some((client, p.clone(), fmt));
                }
            }
            None
        };

        let mut negotiated = if let Some(ref dev) = target_device {
            println!("[bitstream] Testing selected audio device for exclusive/bitstream support...");
            test_device_formats(dev, &profiles).map(|(c, p, f)| (dev.clone(), c, p, f))
        } else if let Some(ref dev) = default_device {
            println!("[bitstream] Testing default audio endpoint for exclusive/bitstream support...");
            test_device_formats(dev, &profiles).map(|(c, p, f)| (dev.clone(), c, p, f))
        } else {
            None
        };

        // If target/default endpoint rejected the formats, probe other active endpoints
        if negotiated.is_none() {
            println!("[bitstream] Default/selected endpoint rejected formats; probing other active endpoints for AVR/HDMI...");
            for i in 0..count {
                if let Ok(dev) = unsafe { collection.Item(i) } {
                    let dev_name = get_device_name(&dev).unwrap_or_default();
                    if let Some((c, p, f)) = test_device_formats(&dev, &profiles) {
                        println!("[bitstream] Selected capable endpoint [{}]: {}", i, dev_name);
                        negotiated = Some((dev, c, p, f));
                        break;
                    }
                }
            }
        }

        let (device, mut audio_client, profile, format) = negotiated
            .ok_or_else(|| anyhow::anyhow!("No exclusive/bitstream format accepted by available audio endpoints.\n\
                Ensure your audio receiver supports multichannel LPCM or bitstream output over HDMI/SPDIF."))?;

        // ── Initialize ──────────────────────────────────────────────
        println!("\n[bitstream] Initializing audio client for {}...", profile.name);
        
        let mut default_period = 0;
        let mut min_period = 0;
        unsafe {
            audio_client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))?;
        }
        
        println!("  Device Periods: Default = {}ns, Min = {}ns", default_period * 100, min_period * 100);

        let mut buffer_duration = if min_period > 0 { min_period } else { default_period.max(100_000) };

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
            if e.code() == windows::core::HRESULT(0x88890019u32 as i32) || e.code() == windows::core::HRESULT(0x80070057u32 as i32) {
                // AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED or E_INVALIDARG
                println!("  Initialize rejected duration {} (HRESULT: {:?}). Attempting to align...", buffer_duration, e.code());
                
                let aligned_frames = unsafe { audio_client.GetBufferSize().unwrap_or(0) };
                if aligned_frames > 0 {
                    buffer_duration = (aligned_frames as i64 * 10_000_000) / profile.rate as i64;
                    println!("  Aligned buffer duration: {}ns ({} frames)", buffer_duration * 100, aligned_frames);
                    
                    let audio_client_new: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
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
        unsafe { audio_client.SetEventHandle(event)?; }

        let buffer_frames = unsafe { audio_client.GetBufferSize()? };
        let bytes_per_sample = (profile.container_bits / 8) as u32;
        let frame_bytes = profile.channels as u32 * bytes_per_sample;
        let render_client: IAudioRenderClient = unsafe { audio_client.GetService()? };
println!("Buffer: {} frames ({:.1} ms)",
            buffer_frames,
            buffer_frames as f64 / profile.rate as f64 * 1000.0);

        // ── Audio & Visualizer Packet Stream Setup ──────────────────
        #[derive(Clone)]
        struct AudioPacket {
            pcm_bytes: Vec<u8>,
            epoch: u64,
            is_eof: bool,
        }

        let (pcm_tx, pcm_rx) = crossbeam_channel::bounded::<AudioPacket>(16);
        let (vis_tx, vis_rx) = crossbeam_channel::bounded::<DspMessage>(128);

        let stop_token_ffmpeg = stop_token.clone();
        let profile_clone = profile.clone();

        let ffmpeg_thread = match stream_type {
            WasapiStreamType::CompressedBitstream => {
                use windows::Win32::System::Pipes::{CreateNamedPipeA, ConnectNamedPipe, NAMED_PIPE_MODE};
                use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

                let pipe_id = PIPE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let pipe_name = format!("\\\\.\\pipe\\rusttracker_bitstream_{}_{}", std::process::id(), pipe_id);
                let pipe_name_nul = format!("{}\0", pipe_name);

                let pipe_handle = unsafe {
                    CreateNamedPipeA(
                        windows::core::PCSTR::from_raw(pipe_name_nul.as_ptr()),
                        FILE_FLAGS_AND_ATTRIBUTES(1), // PIPE_ACCESS_INBOUND
                        NAMED_PIPE_MODE(0), // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT
                        255, // PIPE_UNLIMITED_INSTANCES
                        65536,
                        65536,
                        0,
                        None,
                    )?
                };

                let pipe_name_clone = pipe_name.clone();
                let stop_token_worker = stop_token.clone();
                let vis_tx_worker = vis_tx.clone();

                let ffmpeg_worker = std::thread::spawn(move || {
                    println!("[bitstream] Compressed bitstream FFmpeg worker thread started.");

                    let mut octx = match ffmpeg_next::format::output_as(&pipe_name_clone, "spdif") {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to open spdif muxer: {}", e);
                            let _ = std::fs::OpenOptions::new().write(true).open(&pipe_name_clone);
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
                    let ost_time_base = octx.stream(ost_index).unwrap().time_base();

                    let decoder_context = match ffmpeg_next::codec::context::Context::from_parameters(parameters.clone()) {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create visualizer decoder context: {}", e);
                            return;
                        }
                    };
                    let mut decoder = match decoder_context.decoder().audio() {
                        Ok(dec) => dec,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create visualizer audio decoder: {}", e);
                            return;
                        }
                    };

                    let vis_channels = (decoder.channels() as i32).clamp(2, 8);
                    let target_channel_layout = ffmpeg_next::channel_layout::ChannelLayout::default(vis_channels);
                    let target_ch = vis_channels as usize;

                    let decoder_rate = decoder.rate() as f32;
                    let window_size = crate::audio::calculate_power_of_two_window_size(decoder.rate());
                    let update_interval = ((decoder_rate / 60.0).round() as usize).max(256);
                    let mut accumulator: Vec<Vec<f32>> = vec![vec![0.0f32; window_size]; target_ch];
                    let mut current_seconds = 0.0;

                    let mut resampler: Option<ffmpeg_next::software::resampling::context::Context> = None;
                    let mut cur_in_fmt = ffmpeg_next::format::sample::Sample::None;
                    let mut cur_in_layout = ffmpeg_next::channel_layout::ChannelLayout::default(0);
                    let mut cur_in_rate = 0;
                    
                    for (stream, mut packet) in ictx.packets() {
                        if stop_token_worker.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        if stream.index() == best_audio_index {
                            let vis_packet = packet.clone();

                            let pts = packet.pts().or_else(|| packet.dts());
                            if let Some(p) = pts {
                                current_seconds = p as f64 * f64::from(stream.time_base());
                            }

                            packet.rescale_ts(stream.time_base(), ost_time_base);
                            packet.set_position(-1);
                            packet.set_stream(ost_index);
                            
                            if let Err(e) = packet.write(&mut octx) {
                                eprintln!("[bitstream] Failed to write packet to spdif: {}", e);
                                break;
                            }

                            if decoder.send_packet(&vis_packet).is_ok() {
                                let mut frame = ffmpeg_next::frame::Audio::empty();
                                while decoder.receive_frame(&mut frame).is_ok() {
                                    let frame_layout = if frame.channel_layout().channels() > 0 {
                                        frame.channel_layout()
                                    } else {
                                        ffmpeg_next::channel_layout::ChannelLayout::default(frame.channels().max(1) as i32)
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
                                        let mut vis_frame = ffmpeg_next::frame::Audio::empty();
                                        if resamp.run(&frame, &mut vis_frame).is_ok() {
                                            let total_samples = vis_frame.samples();
                                            let planes = (vis_frame.channels() as usize).min(target_ch);
                                            let mut sample_offset = 0;

                                            while sample_offset < total_samples {
                                                let step = (total_samples - sample_offset).min(update_interval);
                                                for (p, acc) in accumulator.iter_mut().enumerate().take(planes) {
                                                    let plane_data = vis_frame.plane::<f32>(p);
                                                    acc.extend_from_slice(&plane_data[sample_offset..sample_offset + step]);
                                                    let excess = acc.len().saturating_sub(window_size);
                                                    if excess > 0 {
                                                        acc.drain(0..excess);
                                                    }
                                                }

                                                let mut channel_audio_data = Vec::with_capacity(target_ch);
                                                let mut channel_vus = Vec::with_capacity(target_ch);
                                                for acc in accumulator.iter().take(target_ch) {
                                                    let window = acc.clone();
                                                    let mut peak = 0.0f32;
                                                    let start_idx = window.len().saturating_sub(step);
                                                    for &s in &window[start_idx..] {
                                                        peak = peak.max(s.abs());
                                                    }
                                                    channel_vus.push(peak.clamp(0.0, 1.0));
                                                    channel_audio_data.push(window);
                                                }

                                                let mut mono_audio_data = vec![0.0f32; window_size];
                                                for acc in accumulator.iter().take(target_ch) {
                                                    for (i, &sample) in acc.iter().take(window_size).enumerate() {
                                                        mono_audio_data[i] += sample;
                                                    }
                                                }
                                                let inv_ch = 1.0 / target_ch.max(1) as f32;
                                                for s in &mut mono_audio_data {
                                                    *s *= inv_ch;
                                                }

                                                let slice_time = current_seconds + (sample_offset as f64 / decoder.rate() as f64);
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
                                                };

                                                let mut pending = Some(vis_msg);
                                                while let Some(msg) = pending.take() {
                                                    if stop_token_worker.load(std::sync::atomic::Ordering::Relaxed) {
                                                        return;
                                                    }
                                                    match vis_tx_worker.send_timeout(msg, std::time::Duration::from_millis(50)) {
                                                        Ok(()) => break,
                                                        Err(crossbeam_channel::SendTimeoutError::Timeout(m)) => {
                                                            pending = Some(m);
                                                        }
                                                        Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                                                            return;
                                                        }
                                                    }
                                                }
                                                sample_offset += step;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = octx.write_trailer();
                });

                println!("[main] Connecting named pipe for bitstream...");
                unsafe {
                    let _ = ConnectNamedPipe(pipe_handle, None);
                }
                println!("[main] Named pipe connected!");

                let pipe_handle_raw = pipe_handle.0 as isize;
                let pcm_tx_feeder = pcm_tx.clone();
                let stop_token_feeder = stop_token.clone();
                std::thread::spawn(move || {
                    use std::os::windows::io::FromRawHandle;
                    let mut file = unsafe { std::fs::File::from_raw_handle(pipe_handle_raw as *mut std::ffi::c_void) };
                    let mut buf = vec![0u8; 16384];
                    while !stop_token_feeder.load(std::sync::atomic::Ordering::Relaxed) {
                        match file.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let pkt = AudioPacket {
                                    pcm_bytes: buf[..n].to_vec(),
                                    epoch: 0,
                                    is_eof: false,
                                };
                                let mut pending = Some(pkt);
                                while let Some(chunk) = pending.take() {
                                    if stop_token_feeder.load(std::sync::atomic::Ordering::Relaxed) {
                                        return;
                                    }
                                    match pcm_tx_feeder.send_timeout(chunk, std::time::Duration::from_millis(50)) {
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
                            Err(_) => break,
                        }
                    }
                    let _ = pcm_tx_feeder.send(AudioPacket {
                        pcm_bytes: Vec::new(),
                        epoch: 0,
                        is_eof: true,
                    });
                });

                ffmpeg_worker
            }
            WasapiStreamType::Lpcm => {
                let pcm_tx_lpcm = pcm_tx.clone();
                let vis_tx_lpcm = vis_tx.clone();
                let stop_token_lpcm = stop_token_ffmpeg.clone();
                let shared_state_lpcm = shared_state.clone();
                let stream_time_base = f64::from(best_audio.time_base());

                std::thread::spawn(move || {
                    println!("[bitstream] Multi-Channel LPCM FFmpeg worker thread started.");

                    let decoder_context = match ffmpeg_next::codec::context::Context::from_parameters(parameters.clone()) {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create LPCM decoder context: {}", e);
                            return;
                        }
                    };
                    let mut decoder = match decoder_context.decoder().audio() {
                        Ok(dec) => dec,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create LPCM audio decoder: {}", e);
                            return;
                        }
                    };

                    let target_channel_layout = ffmpeg_next::channel_layout::ChannelLayout::default(profile_clone.channels as i32);
                    let target_channels = profile_clone.channels as usize;
                    let bytes_per_sample = (profile_clone.container_bits / 8) as usize;
                    let out_rate = profile_clone.rate as f32;
                    let window_size = crate::audio::calculate_power_of_two_window_size(profile_clone.rate);
                    let update_interval = ((out_rate / 60.0).round() as usize).max(256);
                    let mut accumulator: Vec<Vec<f32>> = vec![vec![0.0f32; window_size]; target_channels];
                    let mut current_seconds = 0.0;

                    let mut pcm_resampler: Option<ffmpeg_next::software::resampling::context::Context> = None;
                    let mut vis_resampler: Option<ffmpeg_next::software::resampling::context::Context> = None;
                    let mut cur_in_fmt = ffmpeg_next::format::sample::Sample::None;
                    let mut cur_in_layout = ffmpeg_next::channel_layout::ChannelLayout::default(0);
                    let mut cur_in_rate = 0;

                    let mut process_frame = |frame: &ffmpeg_next::frame::Audio, current_seconds: f64, epoch: u64| {
                        let frame_layout = if frame.channel_layout().channels() > 0 {
                            frame.channel_layout()
                        } else {
                            ffmpeg_next::channel_layout::ChannelLayout::default(frame.channels().max(1) as i32)
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

                        if let (Some(p_resamp), Some(v_resamp)) = (pcm_resampler.as_mut(), vis_resampler.as_mut()) {
                            let mut pcm_frame = ffmpeg_next::frame::Audio::empty();
                            let mut vis_frame = ffmpeg_next::frame::Audio::empty();
                            let pcm_ok = p_resamp.run(frame, &mut pcm_frame).is_ok();
                            let vis_ok = v_resamp.run(frame, &mut vis_frame).is_ok();

                            if pcm_ok && vis_ok {
                                let total_samples = pcm_frame.samples();
                                let raw_pcm = pcm_frame.data(0);
                                let planes = (vis_frame.channels() as usize).min(target_channels);
                                let mut sample_offset = 0;

                                while sample_offset < total_samples {
                                    let step = (total_samples - sample_offset).min(update_interval);

                                    // Extract PCM slice for hardware
                                    let byte_start = sample_offset * target_channels * bytes_per_sample;
                                    let byte_end = (sample_offset + step) * target_channels * bytes_per_sample;
                                    let mut slice = raw_pcm[byte_start..byte_end.min(raw_pcm.len())].to_vec();
                                    if profile_clone.valid_bits == 24 && profile_clone.container_bits == 32 {
                                        for chunk in slice.as_chunks_mut::<4>().0 {
                                            chunk[0] = 0;
                                        }
                                    }

                                    // Feed accumulator for visualizer
                                    for (p, acc) in accumulator.iter_mut().enumerate().take(planes) {
                                        let plane_data = vis_frame.plane::<f32>(p);
                                        acc.extend_from_slice(&plane_data[sample_offset..sample_offset + step]);
                                        let excess = acc.len().saturating_sub(window_size);
                                        if excess > 0 {
                                            acc.drain(0..excess);
                                        }
                                    }

                                    let mut channel_audio_data = Vec::with_capacity(target_channels);
                                    let mut channel_vus = Vec::with_capacity(target_channels);
                                    for acc in accumulator.iter().take(target_channels) {
                                        let window = acc.clone();
                                        let mut peak = 0.0f32;
                                        let start_idx = window.len().saturating_sub(step);
                                        for &s in &window[start_idx..] {
                                            peak = peak.max(s.abs());
                                        }
                                        channel_vus.push(peak.clamp(0.0, 1.0));
                                        channel_audio_data.push(window);
                                    }

                                    let mut mono_audio_data = vec![0.0f32; window_size];
                                    for acc in accumulator.iter().take(target_channels) {
                                        for (i, &sample) in acc.iter().take(window_size).enumerate() {
                                            mono_audio_data[i] += sample;
                                        }
                                    }
                                    let inv_ch = 1.0 / target_channels.max(1) as f32;
                                    for s in &mut mono_audio_data {
                                        *s *= inv_ch;
                                    }

                                    let slice_time = current_seconds + (sample_offset as f64 / profile_clone.rate as f64);
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
                                    };

                                    let packet = AudioPacket {
                                        pcm_bytes: slice,
                                        epoch,
                                        is_eof: false,
                                    };

                                    let mut pending = Some(packet);
                                    while let Some(pkt) = pending.take() {
                                        if stop_token_lpcm.load(std::sync::atomic::Ordering::Relaxed) {
                                            return;
                                        }
                                        if let Ok(state) = shared_state_lpcm.lock() {
                                            if state.seek_request.is_some() {
                                                return;
                                            }
                                        }
                                        match pcm_tx_lpcm.send_timeout(pkt, std::time::Duration::from_millis(20)) {
                                            Ok(()) => break,
                                            Err(crossbeam_channel::SendTimeoutError::Timeout(c)) => {
                                                pending = Some(c);
                                            }
                                            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                                                return;
                                            }
                                        }
                                    }

                                    let mut pending_vis = Some(vis_msg);
                                    while let Some(msg) = pending_vis.take() {
                                        if stop_token_lpcm.load(std::sync::atomic::Ordering::Relaxed) {
                                            return;
                                        }
                                        if let Ok(state) = shared_state_lpcm.lock() {
                                            if state.seek_request.is_some() {
                                                return;
                                            }
                                        }
                                        match vis_tx_lpcm.send_timeout(msg, std::time::Duration::from_millis(20)) {
                                            Ok(()) => break,
                                            Err(crossbeam_channel::SendTimeoutError::Timeout(m)) => {
                                                pending_vis = Some(m);
                                            }
                                            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                                                return;
                                            }
                                        }
                                    }

                                    sample_offset += step;
                                }
                            }
                        }
                    };

                    let mut current_seek_epoch = 0u64;
                    let mut is_eof = false;

                    while !stop_token_lpcm.load(std::sync::atomic::Ordering::Relaxed) {
                        // 1. Check for seek request from UI
                        let seek_req = {
                            if let Ok(mut state) = shared_state_lpcm.lock() {
                                if let Some(pos) = state.seek_request.take() {
                                    state.seek_epoch += 1;
                                    state.current_seconds = pos;
                                    state.track_ended = false;
                                    Some((pos, state.seek_epoch))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        if let Some((pos, epoch)) = seek_req {
                            current_seek_epoch = epoch;
                            is_eof = false;

                            let target_pts = if stream_time_base > 0.0 { (pos / stream_time_base) as i64 } else { 0 };
                            unsafe {
                                let ret = ffmpeg_next::ffi::av_seek_frame(
                                    ictx.as_mut_ptr(),
                                    best_audio_index as i32,
                                    target_pts,
                                    ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                                );
                                if ret < 0 {
                                    let fallback_pts = (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;
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

                        if is_eof {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            continue;
                        }

                        // 2. Read next packet
                        match ictx.packets().next() {
                            Some((stream, packet)) => {
                                if stream.index() == best_audio_index {
                                    let pts = packet.pts().or_else(|| packet.dts());
                                    if let Some(p) = pts {
                                        current_seconds = p as f64 * stream_time_base;
                                    }

                                    if decoder.send_packet(&packet).is_ok() {
                                        let mut frame = ffmpeg_next::frame::Audio::empty();
                                        while decoder.receive_frame(&mut frame).is_ok() {
                                            if let Ok(state) = shared_state_lpcm.lock() {
                                                if state.seek_request.is_some() {
                                                    break;
                                                }
                                            }
                                            process_frame(&frame, current_seconds, current_seek_epoch);
                                        }
                                    }
                                }
                            }
                            None => {
                                let _ = decoder.send_eof();
                                let mut frame = ffmpeg_next::frame::Audio::empty();
                                while decoder.receive_frame(&mut frame).is_ok() {
                                    process_frame(&frame, current_seconds, current_seek_epoch);
                                }
                                let _ = pcm_tx_lpcm.send(AudioPacket {
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

        // ── Pump loop ───────────────────────────────────────────────
        println!("\n>> Output Active: {} -> {}ch x {}Hz",
            profile.name, profile.channels, profile.rate);

        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}
        impl<T> SendWrapper<T> { fn into_inner(self) -> T { self.0 } }

        let safe_event = SendWrapper(event);
        let safe_audio_client = SendWrapper(audio_client);
        let safe_render_client = SendWrapper(render_client);
        let stop_token_pump = stop_token.clone();
        let shared_state_pump = shared_state.clone();

        let handle = std::thread::spawn(move || {
            let event = safe_event.into_inner();
            let audio_client = safe_audio_client.into_inner();
            let render_client = safe_render_client.into_inner();

            let available = buffer_frames;
            let frame_size = frame_bytes as usize;
            let bytes_needed = (available * frame_bytes) as usize;
            let target_prebuffer = (bytes_needed * 3).max(8192);
            let mut buffer_queue: std::collections::VecDeque<u8> = std::collections::VecDeque::with_capacity(bytes_needed * 8);
            let mut started = false;
            let mut eof = false;
            let mut local_epoch = 0u64;

            // 1. Initial Prebuffering: wait until we have at least target_prebuffer (or EOF)
            let prebuffer_start = std::time::Instant::now();
            while buffer_queue.len() < target_prebuffer && !eof {
                if stop_token_pump.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                match pcm_rx.recv_timeout(std::time::Duration::from_millis(20)) {
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
                        if prebuffer_start.elapsed() > std::time::Duration::from_millis(2000) {
                            break;
                        }
                    }
                }
            }

            let buffer_duration = available as f64 / profile.rate as f64;
            let mut playback_time: Option<f64> = None;

            // Prime the first visualizer message if available
            if let Ok(msg) = vis_rx.try_recv() {
                playback_time = Some(msg.current_seconds);
                let _ = tx.try_send(msg);
            }

            // 2. Playback Loop
            loop {
                if stop_token_pump.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                // Check if seek occurred in shared_state
                let seek_from_state = {
                    if let Ok(state) = shared_state_pump.lock() {
                        if state.seek_epoch > local_epoch {
                            local_epoch = state.seek_epoch;
                            Some(state.current_seconds)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(pos) = seek_from_state {
                    buffer_queue.clear();
                    while pcm_rx.try_recv().is_ok() {}
                    while vis_rx.try_recv().is_ok() {}
                    unsafe {
                        let _ = audio_client.Stop();
                        let _ = audio_client.Reset();
                    }
                    playback_time = Some(pos);
                    started = false;
                    eof = false;
                }

                // If started, wait for WASAPI event indicating buffer space is ready
                if started {
                    let wait_result = unsafe { WaitForSingleObject(event, 50) };
                    if wait_result.0 == 258 /* WAIT_TIMEOUT */ {
                        continue;
                    }
                    if wait_result.0 != WAIT_OBJECT_0 {
                        break;
                    }
                }

                // Replenish buffer_queue up to target_prebuffer — flow controls the decoder!
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
                                    let _ = audio_client.Stop();
                                    let _ = audio_client.Reset();
                                }
                                playback_time = None;
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

                if eof && buffer_queue.is_empty() {
                    // Drain any remaining visualizer frames
                    while let Ok(msg) = vis_rx.try_recv() {
                        let _ = tx.try_send(msg);
                    }
                    if let Ok(mut state) = shared_state_pump.lock() {
                        state.track_ended = true;
                    }
                    break;
                }

                // Strictly align available bytes to frame boundaries
                let aligned_available = (buffer_queue.len() / frame_size) * frame_size;
                let to_copy = bytes_needed.min(aligned_available);

                // If not started yet and we don't have a full buffer, wait for more data rather than outputting partial silence
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
                                    ptr::copy_nonoverlapping(s2.as_ptr(), buf.add(l1), to_copy - l1);
                                }
                                buffer_queue.drain(0..to_copy);
                            }
                            if to_copy < bytes_needed {
                                ptr::write_bytes(buf.add(to_copy), 0, bytes_needed - to_copy);
                            }
                            let flags = if to_copy == 0 { AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 } else { 0 };
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
                } else {
                    // Update playback clock and dispatch visualizers in sync
                    if let Some(pos) = playback_time.as_mut() {
                        *pos += buffer_duration;
                        let target_t = *pos;
                        while let Ok(msg) = vis_rx.try_recv() {
                            let msg_time = msg.current_seconds;
                            let _ = tx.try_send(msg);
                            if msg_time >= target_t {
                                break;
                            }
                        }
                    } else if let Ok(msg) = vis_rx.try_recv() {
                        let t = msg.current_seconds;
                        let _ = tx.try_send(msg);
                        playback_time = Some(t + buffer_duration);
                    }
                }
            }

            drop(pcm_rx);
            drop(vis_rx);
            let _ = ffmpeg_thread.join();
            unsafe {
                let _ = audio_client.Stop();
                let _ = CloseHandle(event);
                CoUninitialize();
            }

            println!("Bitstream/LPCM pump thread finished.");
        });

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

        Ok((handle, out_rate, out_channels, codec_name, has_video))
    }
}

