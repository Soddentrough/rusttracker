use anyhow::Result;
use std::env;

#[cfg(windows)]
mod wasapi_bitstream {
    use anyhow::{Context, Result, bail};
    use std::io::Read;
    use std::process::{Command, Stdio};
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

    #[derive(Debug, Clone)]
    struct Iec61937Profile {
        name: &'static str,
        channels: u16,
        rate: u32,
        sub_format: GUID,
    }

    fn detect_codec_profile(codec_name: &str) -> Vec<Iec61937Profile> {
        if codec_name.contains("truehd") {
            vec![Iec61937Profile {
                name: "TrueHD / Dolby Atmos (MAT 2.0 HBR)",
                channels: 8, rate: 192000,
                sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20,
            }]
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

    fn build_format(profile: &Iec61937Profile) -> WAVEFORMATEXTENSIBLE {
        let block_align = profile.channels * 2;
        let avg_bytes = profile.rate * block_align as u32;
        let channel_mask: u32 = match profile.channels {
            2 => 0x3,
            8 => 0x63F,
            _ => 0x3,
        };
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: 0xFFFE,
                nChannels: profile.channels,
                nSamplesPerSec: profile.rate,
                nAvgBytesPerSec: avg_bytes,
                nBlockAlign: block_align,
                wBitsPerSample: 16,
                cbSize: 22,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: 16 },
            dwChannelMask: channel_mask,
            SubFormat: profile.sub_format,
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

        // ── Probe codec ─────────────────────────────────────────────
        println!("Probing audio stream...");
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "a:0",
                   "-show_entries", "stream=codec_name",
                   "-of", "default=noprint_wrappers=1:nokey=1",
                   file_path])
            .output()
            .context("ffprobe failed. Ensure ffmpeg is in PATH.")?;

        let codec_name = String::from_utf8_lossy(&probe.stdout).trim().to_string();
        if codec_name.is_empty() {
            bail!("Could not detect audio codec. ffprobe stderr:\n{}",
                  String::from_utf8_lossy(&probe.stderr));
        }

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

        let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };

        // ── Negotiate format ────────────────────────────────────────
        println!("\nNegotiating WASAPI Exclusive Mode format...");

        let mut accepted_profile = None;
        let mut accepted_format = None;

        for profile in &profiles {
            let format = build_format(profile);
            let hr = unsafe {
                audio_client.IsFormatSupported(
                    AUDCLNT_SHAREMODE_EXCLUSIVE,
                    &format.Format as *const _,
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
        let buffer_100ns: i64 = 5_000_000;
        unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                buffer_100ns,
                buffer_100ns,
                &format.Format as *const _,
                None,
            )?;
        }

        let event = unsafe { CreateEventW(None, false, false, None)? };
        unsafe { audio_client.SetEventHandle(event)?; }

        let buffer_frames = unsafe { audio_client.GetBufferSize()? };
        let frame_bytes = format.Format.nBlockAlign as u32;
        let render_client: IAudioRenderClient = unsafe { audio_client.GetService()? };

        println!("Buffer: {} frames ({:.1} ms)",
            buffer_frames,
            buffer_frames as f64 / format.Format.nSamplesPerSec as f64 * 1000.0);

        // ── Spawn FFmpeg ────────────────────────────────────────────
        let mut ffmpeg_args = vec![
            "-v", "error",
            "-i", file_path,
            "-c:a", "copy",
        ];
        if codec_name.contains("truehd") {
            ffmpeg_args.extend(["-spdif_flags", "+use_mat"]);
        }
        ffmpeg_args.extend(["-f", "spdif", "-"]);

        println!("\nSpawning: ffmpeg {}", ffmpeg_args.join(" "));

        let mut child = Command::new("ffmpeg")
            .args(&ffmpeg_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn ffmpeg. Ensure ffmpeg.exe is in PATH.")?;

        let mut stdout = child.stdout.take().unwrap();

        // ── Pump loop ───────────────────────────────────────────────
        unsafe { audio_client.Start()?; }
        println!("\n>> Bitstreaming: {} -> {}ch x {}Hz",
            profile.name, profile.channels, profile.rate);
        println!("   Press Ctrl+C to stop.\n");

        let mut total_frames: u64 = 0;
        let mut eof = false;

        loop {
            let wait_result = unsafe { WaitForSingleObject(event, 2000) };
            if wait_result.0 != WAIT_OBJECT_0 {
                eprintln!("WASAPI event timeout.");
                break;
            }

            let padding = unsafe { audio_client.GetCurrentPadding().unwrap_or(0) };
            let available = buffer_frames - padding;
            if available == 0 { continue; }

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

                if total_frames % (format.Format.nSamplesPerSec as u64) < available as u64 {
                    let secs = total_frames as f64 / format.Format.nSamplesPerSec as f64;
                    print!("\r  Streamed: {:.0}s  ", secs);
                }
            }

            if eof { break; }
        }

        let stderr_output = {
            let mut stderr = child.stderr.take().unwrap();
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            buf
        };
        let _ = child.wait();

        unsafe {
            let _ = audio_client.Stop();
            let _ = CloseHandle(event);
            CoUninitialize();
        }

        println!("\n\nPlayback complete. Total frames: {}", total_frames);
        if !stderr_output.is_empty() {
            println!("\nFFmpeg stderr:\n{}", stderr_output);
        }

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
