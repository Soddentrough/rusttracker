#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use crossbeam_channel::Sender;
#[cfg(target_os = "windows")]
use crate::state::AppState;
#[cfg(target_os = "windows")]
use crate::audio::DspMessage;

#[cfg(not(target_os = "windows"))]
pub fn start_bitstream_thread(
    _file_path: &str,
    _shared_state: std::sync::Arc<std::sync::Mutex<crate::state::AppState>>,
    _tx: crossbeam_channel::Sender<crate::audio::DspMessage>,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    Err(anyhow::anyhow!("Bitstreaming is only supported on Windows."))
}



#[cfg(windows)]
pub use wasapi_bitstream::start_bitstream_thread;

#[cfg(windows)]
mod wasapi_bitstream {
    use std::sync::{Arc, Mutex};
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

    #[derive(Debug, Clone)]
    struct Iec61937Profile {
        name: &'static str,
        channels: u16,
        rate: u32,
        sub_format: GUID,
    }

    fn detect_codec_profile(codec_name: &str) -> Vec<Iec61937Profile> {
        if codec_name.contains("truehd") {
            vec![
                Iec61937Profile {
                    name: "TrueHD / Dolby Atmos (MAT 2.0 HBR)",
                    channels: 8, rate: 192000,
                    sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20,
                },
                Iec61937Profile {
                    name: "TrueHD / Dolby Atmos (MLP HBR)",
                    channels: 8, rate: 192000,
                    sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MLP,
                }
            ]
        } else if codec_name.contains("eac3") || codec_name.contains("ec-3") {
            vec![Iec61937Profile {
                name: "E-AC3 / Dolby Digital Plus",
                channels: 2, rate: 192000,
                sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS,
            }]
        } else if codec_name.contains("dts") || codec_name.contains("dca") {
            vec![
                Iec61937Profile {
                    name: "DTS-HD MA (HBR)",
                    channels: 8, rate: 192000,
                    sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD,
                },
                Iec61937Profile {
                    name: "DTS Core (fallback)",
                    channels: 2, rate: 48000,
                    sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DTS,
                },
            ]
        } else if codec_name.contains("ac3") {
            vec![Iec61937Profile {
                name: "AC3 / Dolby Digital",
                channels: 2, rate: 48000,
                sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL,
            }]
        } else {
            vec![Iec61937Profile {
                name: "Unknown (AC3 fallback)",
                channels: 2, rate: 48000,
                sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL,
            }]
        }
    }

    #[repr(C, packed)]
    struct WAVEFORMATEXTENSIBLE_IEC61937 {
        format_ext: WAVEFORMATEXTENSIBLE,
        dw_encoded_samples_per_sec: u32,
        dw_encoded_channel_count: u32,
        dw_average_bytes_per_sec: u32,
    }

    fn build_format(profile: &Iec61937Profile) -> Vec<u8> {
        let block_align = profile.channels * 2;
        let avg_bytes = profile.rate * block_align as u32;
        let channel_mask: u32 = match profile.channels {
            2 => 0x3,
            8 => 0x63F,
            _ => 0x3,
        };

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
            dwChannelMask: channel_mask,
            SubFormat: profile.sub_format,
        };

        if is_hbr {
            let iec = WAVEFORMATEXTENSIBLE_IEC61937 {
                format_ext,
                dw_encoded_samples_per_sec: profile.rate,
                dw_encoded_channel_count: profile.channels as u32,
                dw_average_bytes_per_sec: avg_bytes,
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &iec as *const _ as *const u8,
                    std::mem::size_of::<WAVEFORMATEXTENSIBLE_IEC61937>(),
                )
            };
            bytes.to_vec()
        } else {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &format_ext as *const _ as *const u8,
                    std::mem::size_of::<WAVEFORMATEXTENSIBLE>(),
                )
            };
            bytes.to_vec()
        }
    }

    // ─── Device listing (simple — just count and ID) ─────────────────

    pub fn list_devices() -> Result<()> {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }

        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
        let count = unsafe { collection.GetCount()? };

        println!("Available audio render endpoints:\n");
        for i in 0..count {
            let device = unsafe { collection.Item(i)? };
            let id = unsafe { device.GetId()?.to_string()? };
            println!("  [{}] ID: {}", i, id);
        }
        println!("\nUse --device <N> to select a specific endpoint.");

        unsafe { CoUninitialize(); }
        Ok(())
    }

    // ─── Main bitstream pump ─────────────────────────────────────────

    pub fn start_bitstream_thread(file_path: &str, shared_state: Arc<Mutex<AppState>>, tx: Sender<DspMessage>) -> Result<std::thread::JoinHandle<()>> {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }

        // ── Probe codec via ffmpeg-next ─────────────────────────────
        println!("Probing audio stream via ffmpeg-next...");
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
            _ => "unknown",
        }.to_string();

        let profiles = detect_codec_profile(&codec_name);
        println!("Detected codec: {}", codec_name);
        for p in &profiles {
            println!("  Will try: {} ({}ch x {}Hz)", p.name, p.channels, p.rate);
        }

        // ── Open device ─────────────────────────────────────────────
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };

        let device_idx: Option<u32> = None; let device = if let Some(idx) = device_idx {
            println!("Using device index: {}", idx);
            let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
            unsafe { collection.Item(idx)? }
        } else {
            println!("Using default audio endpoint.");
            unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)? }
        };

        let mut audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };

        // ── Negotiate format ────────────────────────────────────────
        println!("\nNegotiating WASAPI Exclusive Mode format...");

        let mut accepted_profile = None;
        let mut accepted_format = None;

        for profile in &profiles {
            let format = build_format(profile);
            let hr = unsafe {
                audio_client.IsFormatSupported(
                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                    format.as_ptr() as *const _,
                    None,
                )
            };

            if hr.is_ok() {
                println!("  OK: {}", profile.name);
                accepted_profile = Some(profile.clone());
                accepted_format = Some(format);
                break;
            } else {
                println!("  REJECTED: {} (HRESULT: {:?})", profile.name, hr);
            }
        }

        let profile = accepted_profile
            .ok_or_else(|| anyhow::anyhow!("No passthrough format accepted by this endpoint.\n\
                Ensure your default audio device supports bitstream output (HDMI/SPDIF to AVR)."))?;
        let format = accepted_format.unwrap();

        // ── Initialize ──────────────────────────────────────────────
        println!("\nInitializing audio client...");
        
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
                
                // If it failed with E_INVALIDARG, we might need a new client, but let's try querying buffer size first
                let aligned_frames = unsafe { audio_client.GetBufferSize().unwrap_or(0) };
                if aligned_frames > 0 {
                    // formula: duration = frames * 10_000_000 / sample_rate
                    buffer_duration = (aligned_frames as i64 * 10_000_000) / profile.rate as i64;
                    println!("  Aligned buffer duration: {}ns ({} frames)", buffer_duration * 100, aligned_frames);
                    
                    // We actually must get a new audio_client instance if Initialize failed
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
        let frame_bytes = (profile.channels * 2) as u32;
        let render_client: IAudioRenderClient = unsafe { audio_client.GetService()? };

        println!("Buffer: {} frames ({:.1} ms)",
            buffer_frames,
            buffer_frames as f64 / profile.rate as f64 * 1000.0);

        // ── Start FFmpeg spdif Muxer Pipe ───────────────────────────
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
        let best_audio_index = best_audio.index();
        let parameters = best_audio.parameters();

        let ffmpeg_thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            println!("[bitstream] FFmpeg thread started.");

            let mut octx = ffmpeg_next::format::output_as(&pipe_name_clone, "spdif").unwrap();
            let ost_index = {
                let mut ost = octx.add_stream(ffmpeg_next::codec::Id::None).unwrap();
                ost.set_parameters(parameters.clone());
                ost.index()
            };
            
            let mut dict = ffmpeg_next::Dictionary::new();
            dict.set("flush_packets", "1");
            octx.write_header_with(dict).unwrap();
            let ost_time_base = octx.stream(ost_index).unwrap().time_base();

            // Setup Visualizer Decoder
            let decoder_context = ffmpeg_next::codec::context::Context::from_parameters(parameters.clone()).unwrap();
            let mut decoder = decoder_context.decoder().audio().unwrap();
            let mut resampler = ffmpeg_next::software::resampling::context::Context::get(
                decoder.format(), decoder.channel_layout(), decoder.rate(),
                ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
                decoder.channel_layout(), decoder.rate(),
            ).unwrap();

            let decoder_rate = decoder.rate() as f32;
            let window_size = (((decoder_rate * 0.185).round() as usize) / 2) * 2;
            let window_size = window_size.max(2048).min(65536);
            let mut accumulator: Vec<Vec<f32>> = Vec::new();

            let mut pkts_written = 0;
            let mut current_seconds = 0.0;
            
            for (stream, mut packet) in ictx.packets() {
                if stream.index() == best_audio_index {
                    let vis_packet = packet.clone();

                    if let Some(pts) = packet.pts() {
                        current_seconds = pts as f64 * f64::from(stream.time_base());
                    }

                    packet.rescale_ts(stream.time_base(), ost_time_base);
                    packet.set_stream(ost_index);
                    let _ = packet.write(&mut octx);
                    pkts_written += 1;

                    if decoder.send_packet(&vis_packet).is_ok() {
                        let mut frame = ffmpeg_next::frame::Audio::empty();
                        while decoder.receive_frame(&mut frame).is_ok() {
                            let mut resampled = ffmpeg_next::frame::Audio::empty();
                            if resampler.run(&frame, &mut resampled).is_ok() {
                                let planes = resampled.channels() as usize;
                                
                                if accumulator.len() != planes {
                                    accumulator = vec![Vec::new(); planes];
                                }
                                
                                for p in 0..planes {
                                    let data = resampled.plane::<f32>(p);
                                    accumulator[p].extend_from_slice(data);
                                    let excess = accumulator[p].len().saturating_sub(window_size);
                                    if excess > 0 {
                                        accumulator[p].drain(0..excess);
                                    }
                                }
                                
                                if accumulator.get(0).map(|a| a.len()).unwrap_or(0) == window_size {
                                    let mut channel_audio_data = Vec::with_capacity(planes);
                                    let mut channel_vus = Vec::with_capacity(planes);
                                    
                                    for p in 0..planes {
                                        let window = accumulator[p].clone();
                                        
                                        // Calculate simple RMS for VU using only the fresh samples from this frame
                                        let mut sum_sq = 0.0;
                                        let fresh_data = resampled.plane::<f32>(p);
                                        for &s in fresh_data { sum_sq += s * s; }
                                        let rms = (sum_sq / fresh_data.len().max(1) as f32).sqrt();
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
        });

        println!("[main] Connecting named pipe...");
        unsafe {
            let _ = ConnectNamedPipe(pipe_handle, None);
        }
        println!("[main] Named pipe connected!");

        use std::os::windows::io::FromRawHandle;
        let mut stdout = unsafe { std::fs::File::from_raw_handle(pipe_handle.0 as _) };

        // ── Pump loop ───────────────────────────────────────────────
        println!("\n>> Bitstreaming: {} -> {}ch x {}Hz",
            profile.name, profile.channels, profile.rate);
        println!("   Press Ctrl+C to stop.\n");

        let mut total_frames: u64 = 0;
        let mut eof = false;
        let mut started = false;

        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}
        impl<T> SendWrapper<T> { fn into_inner(self) -> T { self.0 } }

        let safe_event = SendWrapper(event);
        let safe_pipe = SendWrapper(pipe_handle);
        let safe_audio_client = SendWrapper(audio_client);
        let safe_render_client = SendWrapper(render_client);

        let handle = std::thread::spawn(move || {
            let event = safe_event.into_inner();
            let pipe_handle = safe_pipe.into_inner();
            let audio_client = safe_audio_client.into_inner();
            let render_client = safe_render_client.into_inner();
            loop {
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
            
            println!("Bitstream thread finished.");
        });

        Ok(handle)
    }
}

