use anyhow::Result;
use std::env;

#[cfg(windows)]
mod wasapi_bitstream {
    use anyhow::{Context, Result, bail};
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

    pub fn run(file_path: &str, device_idx: Option<u32>) -> Result<()> {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }

        // ── Probe codec via ffmpeg-next ─────────────────────────────
        println!("Probing audio stream via ffmpeg-next...");
        ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
        ffmpeg_next::init().context("Failed to initialize ffmpeg-next")?;
        
        let mut ictx = ffmpeg_next::format::input(&file_path)
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

        let device = if let Some(idx) = device_idx {
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
            // Wait slightly for server to block on ConnectNamedPipe
            std::thread::sleep(std::time::Duration::from_millis(50));
            println!("[ffmpeg] Thread started, opening output...");

            let mut octx = match ffmpeg_next::format::output_as(&pipe_name_clone, "spdif") {
                Ok(c) => c,
                Err(e) => { eprintln!("[ffmpeg] Output error: {:?}", e); return; }
            };

            let ost_index = {
                let mut ost = octx.add_stream(ffmpeg_next::codec::Id::None).unwrap();
                ost.set_parameters(parameters);
                ost.index()
            };
            
            let mut dict = ffmpeg_next::Dictionary::new();
            dict.set("flush_packets", "1");
            println!("[ffmpeg] Writing header...");
            octx.write_header_with(dict).unwrap();
            println!("[ffmpeg] Header written!");

            let ost_time_base = octx.stream(ost_index).unwrap().time_base();

            let mut pkts_written = 0;
            for (stream, mut packet) in ictx.packets() {
                if stream.index() == best_audio_index {
                    packet.rescale_ts(stream.time_base(), ost_time_base);
                    packet.set_stream(ost_index);
                    
                    let _ = packet.write(&mut octx);
                    pkts_written += 1;
                }
            }
            println!("[ffmpeg] Finished writing {} packets. Writing trailer...", pkts_written);
            let _ = octx.write_trailer();
            println!("[ffmpeg] Thread exiting.");
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

        loop {
            if started {
                let wait_result = unsafe { WaitForSingleObject(event, 2000) };
                if wait_result.0 != WAIT_OBJECT_0 {
                    eprintln!("WASAPI event timeout.");
                    break;
                }
            }

            // In Exclusive Event-Driven mode, we must always provide a full buffer
            let available = buffer_frames;

            let bytes_needed = (available * frame_bytes) as usize;
            let mut chunk = vec![0u8; bytes_needed];
            let mut filled = 0;

            if !started {
                println!("[main] Reading first buffer: need {} bytes", bytes_needed);
            }

            while filled < bytes_needed && !eof {
                match stdout.read(&mut chunk[filled..]) {
                    Ok(0) => { 
                        if !started { println!("[main] EOF reached on first read!"); }
                        eof = true; 
                    }
                    Ok(n) => { 
                        if !started { println!("[main] Read {} bytes from pipe", n); }
                        filled += n; 
                    }
                    Err(e) => { 
                        eprintln!("[main] Pipe read error: {}", e);
                        eof = true; 
                    }
                }
            }

            if eof && filled == 0 {
                println!("\nEnd of stream reached.");
                break;
            }

            if filled > 0 {
                unsafe {
                    let buf = render_client.GetBuffer(available)?;
                    ptr::copy_nonoverlapping(chunk.as_ptr(), buf, filled);
                    if filled < bytes_needed {
                        ptr::write_bytes(buf.add(filled), 0, bytes_needed - filled);
                    }
                    render_client.ReleaseBuffer(available, 0)?;
                }
                total_frames += available as u64;

                if total_frames % (profile.rate as u64) < available as u64 {
                    let secs = total_frames as f64 / profile.rate as f64;
                    use std::io::Write;
                    print!("\r  Streamed: {:.0}s  ", secs);
                    let _ = std::io::stdout().flush();
                }
            }
            
            if !started && filled > 0 {
                println!("[main] Starting WASAPI audio client!");
                unsafe { audio_client.Start()?; }
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

        println!("\n\nPlayback complete. Total frames: {}", total_frames);

        Ok(())
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    println!("RustTracker Bitstream Passthrough Tester");
    println!("========================================\n");

    if args.len() < 2 {
        println!("Usage:");
        println!("  {} <file>                  Bitstream to default device", args[0]);
        println!("  {} <file> --device <N>     Bitstream to device index N", args[0]);
        println!("  {} --list-devices          List audio endpoints\n", args[0]);
        println!("Requires: ffmpeg.exe and ffprobe.exe in PATH.");
        return Ok(());
    }

    #[cfg(windows)]
    {
        if args.iter().any(|a| a == "--list-devices") {
            return wasapi_bitstream::list_devices();
        }

        let file_path = &args[1];
        let device_idx = args.iter().position(|a| a == "--device")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u32>().ok());

        wasapi_bitstream::run(file_path, device_idx)?;
    }

    #[cfg(not(windows))]
    {
        eprintln!("This tool only runs on Windows (WASAPI Exclusive Mode).");
        eprintln!("For Linux, use the ALSA version in scratch_bitstream/.");
    }

    Ok(())
}
