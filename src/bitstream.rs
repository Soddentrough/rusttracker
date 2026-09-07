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

        let decoder_context = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone()).unwrap();
        let mut decoder = decoder_context.decoder().audio().unwrap();
        let mut resampler = ffmpeg_next::software::resampling::context::Context::get(
            decoder.format(), decoder.channel_layout(), decoder.rate(),
            ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
            decoder.channel_layout(), decoder.rate(),
        ).unwrap();

        let decoder_rate = decoder.rate() as f32;
        let window_size = (((decoder_rate * 0.185).round() as usize) / 2) * 2;
        let window_size = window_size.clamp(2048, 65536);
        let update_interval = (decoder_rate / 240.0).ceil() as usize;
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
                                
                                for window in &accumulator {
                                    let mut sum_sq = 0.0;
                                    for &s in window { sum_sq += s * s; }
                                    let rms = (sum_sq / window.len() as f32).sqrt();
                                    channel_vus.push(rms);
                                    channel_audio_data.push(window.clone());
                                }
                                
                                let _ = tx.try_send(crate::audio::DspMessage {
                                    audio_data: channel_audio_data[0].clone(),
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
    use std::io::{Read, Write};
    use std::ptr;
    use windows::core::GUID;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    const WAIT_OBJECT_0: u32 = 0;

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
        _shared_state: Arc<Mutex<AppState>>,
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

        let selected_device_name = _shared_state.lock().ok().and_then(|s| s.selected_audio_device.clone());
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

        let mut buffer_duration = min_period;

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

        // ── Start Named Pipe for streaming ───────────────────────────
        use windows::Win32::System::Pipes::{CreateNamedPipeA, ConnectNamedPipe, NAMED_PIPE_MODE};
        use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
        
        let pipe_name = format!("\\\\.\\pipe\\rusttracker_bitstream_{}", std::process::id());
        let pipe_name_nul = format!("{}\0", pipe_name);

        let pipe_handle = unsafe {
            CreateNamedPipeA(
                windows::core::PCSTR::from_raw(pipe_name_nul.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(1), // PIPE_ACCESS_INBOUND
                NAMED_PIPE_MODE(0), // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT
                1,
                65536,
                65536,
                0,
                None,
            )?
        };

        let pipe_name_clone = pipe_name.clone();
        let stop_token_ffmpeg = stop_token.clone();
        let profile_clone = profile.clone();

        let ffmpeg_thread = match stream_type {
            WasapiStreamType::CompressedBitstream => {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    println!("[bitstream] Compressed bitstream FFmpeg worker thread started.");

                    let mut octx = match ffmpeg_next::format::output_as(&pipe_name_clone, "spdif") {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to open spdif muxer: {}", e);
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

                    // Setup Visualizer Decoder
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
                    let mut resampler = match ffmpeg_next::software::resampling::context::Context::get(
                        decoder.format(), decoder.channel_layout(), decoder.rate(),
                        ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
                        decoder.channel_layout(), decoder.rate(),
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create visualizer resampler: {}", e);
                            return;
                        }
                    };

                    let decoder_rate = decoder.rate() as f32;
                    let window_size = (((decoder_rate * 0.185).round() as usize) / 2) * 2;
                    let window_size = window_size.max(2048).min(65536);
                    let update_interval = (decoder_rate / 240.0).ceil() as usize;
                    let mut accumulator: Vec<Vec<f32>> = Vec::new();
                    let mut samples_since_last_send = 0;
                    let mut current_seconds = 0.0;
                    
                    for (stream, mut packet) in ictx.packets() {
                        if stop_token_ffmpeg.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        if stream.index() == best_audio_index {
                            let vis_packet = packet.clone();

                            if let Some(pts) = packet.pts() {
                                current_seconds = pts as f64 * f64::from(stream.time_base());
                            }

                            packet.rescale_ts(stream.time_base(), ost_time_base);
                            packet.set_stream(ost_index);
                            let _ = packet.write(&mut octx);

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
                                        for p in 0..planes {
                                            let data = resampled.plane::<f32>(p);
                                            if p == 0 { fresh_samples = data.len(); }
                                            accumulator[p].extend_from_slice(data);
                                            let excess = accumulator[p].len().saturating_sub(window_size);
                                            if excess > 0 {
                                                accumulator[p].drain(0..excess);
                                            }
                                        }
                                        
                                        samples_since_last_send += fresh_samples;
                                        
                                        if accumulator.first().map(|a| a.len()).unwrap_or(0) == window_size && samples_since_last_send >= update_interval {
                                            samples_since_last_send = 0;
                                            let mut channel_audio_data = Vec::with_capacity(planes);
                                            let mut channel_vus = Vec::with_capacity(planes);
                                            
                                            for p in 0..planes {
                                                let window = accumulator[p].clone();
                                                let mut sum_sq = 0.0;
                                                for &s in &window { sum_sq += s * s; }
                                                let rms = (sum_sq / window.len() as f32).sqrt();
                                                channel_vus.push(rms);
                                                channel_audio_data.push(window);
                                            }
                                            
                                            let _ = tx.try_send(DspMessage {
                                                audio_data: channel_audio_data[0].clone(),
                                                channel_vus,
                                                current_order: 0,
                                                current_row: 0,
                                                bpm: 0,
                                                speed: 0,
                                                current_seconds,
                                                current_row_string: String::new(),
                                                channel_audio_data,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = octx.write_trailer();
                })
            }
            WasapiStreamType::Lpcm => {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    println!("[bitstream] Multi-Channel LPCM FFmpeg worker thread started.");

                    let mut pipe_writer = match std::fs::OpenOptions::new().write(true).open(&pipe_name_clone) {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to open pipe for writing: {}", e);
                            return;
                        }
                    };

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
                    let mut pcm_resampler = match ffmpeg_next::software::resampling::context::Context::get(
                        decoder.format(),
                        decoder.channel_layout(),
                        decoder.rate(),
                        profile_clone.sample_format,
                        target_channel_layout,
                        profile_clone.rate,
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create PCM resampler: {}", e);
                            return;
                        }
                    };

                    let mut vis_resampler = match ffmpeg_next::software::resampling::context::Context::get(
                        decoder.format(),
                        decoder.channel_layout(),
                        decoder.rate(),
                        ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
                        decoder.channel_layout(),
                        decoder.rate(),
                    ) {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!("[bitstream] Failed to create visualizer resampler: {}", e);
                            return;
                        }
                    };

                    let decoder_rate = decoder.rate() as f32;
                    let window_size = (((decoder_rate * 0.185).round() as usize) / 2) * 2;
                    let window_size = window_size.max(2048).min(65536);
                    let update_interval = (decoder_rate / 240.0).ceil() as usize;
                    let mut accumulator: Vec<Vec<f32>> = Vec::new();
                    let mut samples_since_last_send = 0;
                    let mut current_seconds = 0.0;

                    for (stream, packet) in ictx.packets() {
                        if stop_token_ffmpeg.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        if stream.index() == best_audio_index {
                            if let Some(pts) = packet.pts() {
                                current_seconds = pts as f64 * f64::from(stream.time_base());
                            }

                            if decoder.send_packet(&packet).is_ok() {
                                let mut frame = ffmpeg_next::frame::Audio::empty();
                                while decoder.receive_frame(&mut frame).is_ok() {
                                    // 1. Output packed PCM bytes to WASAPI pipe
                                    let mut pcm_frame = ffmpeg_next::frame::Audio::empty();
                                    if pcm_resampler.run(&frame, &mut pcm_frame).is_ok() {
                                        let samples = pcm_frame.samples();
                                        let ch = pcm_frame.channels() as usize;
                                        let bytes_per_sample = (profile_clone.container_bits / 8) as usize;
                                        let total_bytes = samples * ch * bytes_per_sample;
                                        if total_bytes > 0 {
                                            let raw_bytes = pcm_frame.data(0);
                                            let slice = &raw_bytes[..total_bytes.min(raw_bytes.len())];
                                            if pipe_writer.write_all(slice).is_err() {
                                                break;
                                            }
                                        }
                                    }

                                    // 2. Feed visualizer DSP
                                    let mut vis_frame = ffmpeg_next::frame::Audio::empty();
                                    if vis_resampler.run(&frame, &mut vis_frame).is_ok() {
                                        let planes = vis_frame.channels() as usize;
                                        if accumulator.len() != planes {
                                            accumulator = vec![Vec::new(); planes];
                                        }
                                        let mut fresh_samples = 0;
                                        for p in 0..planes {
                                            let data = vis_frame.plane::<f32>(p);
                                            if p == 0 { fresh_samples = data.len(); }
                                            accumulator[p].extend_from_slice(data);
                                            let excess = accumulator[p].len().saturating_sub(window_size);
                                            if excess > 0 {
                                                accumulator[p].drain(0..excess);
                                            }
                                        }
                                        samples_since_last_send += fresh_samples;
                                        if accumulator.first().map(|a| a.len()).unwrap_or(0) == window_size && samples_since_last_send >= update_interval {
                                            samples_since_last_send = 0;
                                            let mut channel_audio_data = Vec::with_capacity(planes);
                                            let mut channel_vus = Vec::with_capacity(planes);
                                            for p in 0..planes {
                                                let window = accumulator[p].clone();
                                                let mut sum_sq = 0.0;
                                                for &s in &window { sum_sq += s * s; }
                                                let rms = (sum_sq / window.len() as f32).sqrt();
                                                channel_vus.push(rms);
                                                channel_audio_data.push(window);
                                            }
                                            let _ = tx.try_send(DspMessage {
                                                audio_data: channel_audio_data[0].clone(),
                                                channel_vus,
                                                current_order: 0,
                                                current_row: 0,
                                                bpm: 0,
                                                speed: 0,
                                                current_seconds,
                                                current_row_string: String::new(),
                                                channel_audio_data,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Flush decoder
                    let _ = decoder.send_eof();
                    let mut frame = ffmpeg_next::frame::Audio::empty();
                    while decoder.receive_frame(&mut frame).is_ok() {
                        let mut pcm_frame = ffmpeg_next::frame::Audio::empty();
                        if pcm_resampler.run(&frame, &mut pcm_frame).is_ok() {
                            let samples = pcm_frame.samples();
                            let ch = pcm_frame.channels() as usize;
                            let bytes_per_sample = (profile_clone.container_bits / 8) as usize;
                            let total_bytes = samples * ch * bytes_per_sample;
                            if total_bytes > 0 {
                                let raw_bytes = pcm_frame.data(0);
                                let slice = &raw_bytes[..total_bytes.min(raw_bytes.len())];
                                if pipe_writer.write_all(slice).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    let _ = pipe_writer.flush();
                })
            }
        };

        println!("[main] Connecting named pipe...");
        unsafe {
            let _ = ConnectNamedPipe(pipe_handle, None);
        }
        println!("[main] Named pipe connected!");

        use std::os::windows::io::FromRawHandle;
        let mut stdout = unsafe { std::fs::File::from_raw_handle(pipe_handle.0 as _) };

        // ── Pump loop ───────────────────────────────────────────────
        println!("\n>> Output Active: {} -> {}ch x {}Hz",
            profile.name, profile.channels, profile.rate);

        let mut eof = false;
        let mut started = false;

        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}
        impl<T> SendWrapper<T> { fn into_inner(self) -> T { self.0 } }

        let safe_event = SendWrapper(event);
        let safe_pipe = SendWrapper(pipe_handle);
        let safe_audio_client = SendWrapper(audio_client);
        let safe_render_client = SendWrapper(render_client);
        let stop_token_pump = stop_token.clone();

        let handle = std::thread::spawn(move || {
            let event = safe_event.into_inner();
            let pipe_handle = safe_pipe.into_inner();
            let audio_client = safe_audio_client.into_inner();
            let render_client = safe_render_client.into_inner();
            loop {
                if stop_token_pump.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                if started {
                    let wait_result = unsafe { WaitForSingleObject(event, 2000) };
                    if wait_result.0 != WAIT_OBJECT_0 { break; }
                }

                let available = buffer_frames;
                let bytes_needed = (available * frame_bytes) as usize;
                let mut chunk = vec![0u8; bytes_needed];
                let mut filled = 0;

                while filled < bytes_needed && !eof {
                    match stdout.read(&mut chunk[filled..]) {
                        Ok(0) => { eof = true; }
                        Ok(n) => { filled += n; }
                        Err(_) => { eof = true; }
                    }
                }

                if eof && filled == 0 { break; }

                if filled > 0 {
                    unsafe {
                        let buf = render_client.GetBuffer(available).unwrap();
                        ptr::copy_nonoverlapping(chunk.as_ptr(), buf, filled);
                        if filled < bytes_needed {
                            ptr::write_bytes(buf.add(filled), 0, bytes_needed - filled);
                        }
                        render_client.ReleaseBuffer(available, 0).unwrap();
                    }
                }
                
                if !started && filled > 0 {
                    unsafe { audio_client.Start().unwrap(); }
                    started = true;
                }
            }

            let _ = ffmpeg_thread.join();
            unsafe {
                let _ = audio_client.Stop();
                let _ = CloseHandle(event);
                let _ = windows::Win32::Foundation::CloseHandle(pipe_handle);
                CoUninitialize();
            }
            
            println!("Bitstream/LPCM pump thread finished.");
        });

        Ok((handle, decoder_sample_rate, profile.channels, codec_name, has_video))
    }
}

