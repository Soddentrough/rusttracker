use anyhow::{Context, Result, bail};
use std::env;
use std::io::Read;
use std::process::{Command, Stdio};
use std::ptr;

#[cfg(windows)]
use windows::core::GUID;
#[cfg(windows)]
use windows::Win32::Media::Audio::*;
#[cfg(windows)]
use windows::Win32::System::Com::*;
#[cfg(windows)]
use windows::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, WAIT_OBJECT_0};

// ─── IEC 61937 codec table ───────────────────────────────────────────
// Each entry maps an FFmpeg codec name to the IEC 61937 carrier parameters
// required by the WASAPI Exclusive Mode endpoint.

#[derive(Debug, Clone)]
struct Iec61937Profile {
    name: &'static str,
    channels: u16,       // IEC carrier channels (2 = standard, 8 = HBR)
    rate: u32,           // carrier sample rate
    #[cfg(windows)]
    sub_format: GUID,    // WASAPI SubFormat GUID
}

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL: GUID = GUID {
    data1: 0x00000092, data2: 0x0000, data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS: GUID = GUID {
    data1: 0x0000000a, data2: 0xcea, data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20: GUID = GUID {
    data1: 0x00000017, data2: 0xcea, data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_IEC61937_DTS: GUID = GUID {
    data1: 0x00000008, data2: 0x0000, data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD: GUID = GUID {
    data1: 0x0000000b, data2: 0xcea, data3: 0x0010,
    data4: [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
};

#[cfg(windows)]
fn detect_codec_profile(codec_name: &str) -> Iec61937Profile {
    if codec_name.contains("truehd") {
        Iec61937Profile {
            name: "TrueHD / Dolby Atmos (MAT 2.0 HBR)",
            channels: 8,
            rate: 192000,
            sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MAT20,
        }
    } else if codec_name.contains("eac3") || codec_name.contains("ec-3") {
        Iec61937Profile {
            name: "E-AC3 / Dolby Digital Plus",
            channels: 2,
            rate: 192000,
            sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS,
        }
    } else if codec_name.contains("dts") || codec_name.contains("dca") {
        // DTS-HD MA uses HBR, core DTS uses standard 2ch/48kHz.
        // We try HBR first; if the endpoint rejects it we fall back.
        Iec61937Profile {
            name: "DTS-HD MA (HBR)",
            channels: 8,
            rate: 192000,
            sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD,
        }
    } else if codec_name.contains("ac3") {
        Iec61937Profile {
            name: "AC3 / Dolby Digital",
            channels: 2,
            rate: 48000,
            sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL,
        }
    } else {
        // Unknown — try AC3 carrier as safest default
        Iec61937Profile {
            name: "Unknown (AC3 fallback)",
            channels: 2,
            rate: 48000,
            sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL,
        }
    }
}

#[cfg(windows)]
fn get_dts_core_fallback() -> Iec61937Profile {
    Iec61937Profile {
        name: "DTS Core (standard rate fallback)",
        channels: 2,
        rate: 48000,
        sub_format: KSDATAFORMAT_SUBTYPE_IEC61937_DTS,
    }
}

// ─── Device enumeration ──────────────────────────────────────────────

#[cfg(windows)]
fn list_audio_endpoints() -> Result<Vec<(String, String)>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        
        let mut devices = Vec::new();
        for i in 0..count {
            let device = collection.Item(i)?;
            let id = device.GetId()?.to_string()?;
            
            let props = device.OpenPropertyStore(windows::Win32::System::Com::StructuredStorage::STGM_READ)?;
            let friendly_name = {
                let key = windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
                    fmtid: GUID { data1: 0xa45c254e, data2: 0xdf1c, data3: 0x4efd, data4: [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0] },
                    pid: 14,
                };
                match props.GetValue(&key) {
                    Ok(val) => format!("{}", val.to_string()),
                    Err(_) => format!("Device {}", i),
                }
            };
            
            devices.push((id, friendly_name));
        }
        Ok(devices)
    }
}

// ─── WASAPI Exclusive Mode bitstream pump ────────────────────────────

#[cfg(windows)]
fn try_exclusive_format(
    audio_client: &IAudioClient,
    profile: &Iec61937Profile,
) -> Result<WAVEFORMATEXTENSIBLE> {
    let block_align = profile.channels * 2; // 16-bit samples
    let avg_bytes = profile.rate * block_align as u32;
    
    let channel_mask: u32 = match profile.channels {
        2 => 0x3,       // FL | FR
        8 => 0x63F,     // FL|FR|FC|LFE|BL|BR|SL|SR
        _ => 0x3,
    };
    
    let format = WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xFFFE, // WAVE_FORMAT_EXTENSIBLE
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
    };
    
    unsafe {
        audio_client.IsFormatSupported(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            &format.Format as *const _ as _,
            None,
        )?;
    }
    
    Ok(format)
}

#[cfg(windows)]
fn run_bitstream(file_path: &str, device_id: Option<&str>) -> Result<()> {
    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
    
    // ── Step 1: Probe the file ──────────────────────────────────────
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
        bail!("Could not detect audio codec in file.");
    }
    
    let mut profile = detect_codec_profile(&codec_name);
    println!("Detected: {} (codec: {})", profile.name, codec_name);
    println!("Carrier: {}ch × {} Hz × 16-bit", profile.channels, profile.rate);
    
    // ── Step 2: Open WASAPI device ──────────────────────────────────
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    
    let device = if let Some(id) = device_id {
        println!("Opening device: {}", id);
        let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { enumerator.GetDevice(windows::core::PCWSTR(wide.as_ptr()))? }
    } else {
        println!("Using default audio endpoint.");
        unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)? }
    };
    
    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };
    
    // ── Step 3: Negotiate format ────────────────────────────────────
    println!("Requesting WASAPI Exclusive Mode...");
    let format = match try_exclusive_format(&audio_client, &profile) {
        Ok(fmt) => {
            println!("  ✓ Format accepted: {}", profile.name);
            fmt
        }
        Err(e) => {
            println!("  ✗ Rejected: {} ({:?})", profile.name, e);
            
            // DTS-HD → try DTS Core fallback
            if codec_name.contains("dts") || codec_name.contains("dca") {
                let fallback = get_dts_core_fallback();
                println!("  Trying fallback: {}", fallback.name);
                profile = fallback;
                match try_exclusive_format(&audio_client, &profile) {
                    Ok(fmt) => {
                        println!("  ✓ Fallback accepted.");
                        fmt
                    }
                    Err(e2) => bail!("No supported passthrough format. Last error: {:?}", e2),
                }
            } else {
                bail!("Endpoint does not support passthrough for {}. Error: {:?}", profile.name, e);
            }
        }
    };
    
    // ── Step 4: Initialize audio client ─────────────────────────────
    // Use 500ms buffer. WASAPI may align this; that's fine.
    let buffer_100ns: i64 = 5_000_000;
    unsafe {
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            buffer_100ns,
            buffer_100ns,
            &format.Format as *const _ as _,
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
    
    // ── Step 5: Spawn FFmpeg spdif muxer ────────────────────────────
    // Build the ffmpeg command to output the correct spdif carrier.
    // For TrueHD HBR we need `-spdif_flags +use_mat` to get MAT 2.0 framing.
    let mut ffmpeg_args = vec![
        "-v".to_string(), "error".to_string(),
        "-i".to_string(), file_path.to_string(),
        "-c:a".to_string(), "copy".to_string(),
    ];
    
    // TrueHD needs special spdif flags for MAT encapsulation
    if codec_name.contains("truehd") {
        ffmpeg_args.extend(["-spdif_flags".to_string(), "+use_mat".to_string()]);
    }
    
    ffmpeg_args.extend(["-f".to_string(), "spdif".to_string(), "-".to_string()]);
    
    println!("Spawning: ffmpeg {}", ffmpeg_args.join(" "));
    
    let mut child = Command::new("ffmpeg")
        .args(&ffmpeg_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn ffmpeg")?;
    
    let mut stdout = child.stdout.take().unwrap();
    
    // ── Step 6: Pump loop ───────────────────────────────────────────
    unsafe { audio_client.Start()?; }
    println!("\n▶ Bitstreaming active. Press Ctrl+C to stop.\n");
    
    let mut total_frames: u64 = 0;
    let mut eof = false;
    
    loop {
        let wait_result = unsafe { WaitForSingleObject(event, 2000) };
        if wait_result != WAIT_OBJECT_0 {
            eprintln!("WASAPI event timeout — device may have been disconnected.");
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
            
            // Progress indicator
            let secs = total_frames as f64 / format.Format.nSamplesPerSec as f64;
            if total_frames % (format.Format.nSamplesPerSec as u64) < available as u64 {
                print!("\r  Streamed: {:.0}s", secs);
            }
        }
        
        if eof { break; }
    }
    
    println!("\n\nPlayback complete. Total frames: {}", total_frames);
    
    let _ = child.wait();
    unsafe {
        let _ = audio_client.Stop();
        let _ = CloseHandle(event);
    }
    
    Ok(())
}

// ─── Main ────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        println!("RustTracker Bitstream Passthrough Tester");
        println!("========================================\n");
        println!("Usage:");
        println!("  {} <file>                    Bitstream to default device", args[0]);
        println!("  {} <file> --device <id>      Bitstream to specific device", args[0]);
        println!("  {} --list-devices            List available audio endpoints\n", args[0]);
        return Ok(());
    }
    
    #[cfg(windows)]
    {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
        
        if args.iter().any(|a| a == "--list-devices") {
            println!("Available audio endpoints:\n");
            let devices = list_audio_endpoints()?;
            for (i, (id, name)) in devices.iter().enumerate() {
                println!("  [{}] {}", i, name);
                println!("      ID: {}\n", id);
            }
            return Ok(());
        }
        
        let file_path = &args[1];
        let device_id = args.iter().position(|a| a == "--device")
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str());
        
        run_bitstream(file_path, device_id)?;
    }
    
    #[cfg(not(windows))]
    {
        eprintln!("This test program only runs on Windows (WASAPI Exclusive Mode).");
        eprintln!("For Linux, use the ALSA version in scratch_bitstream/.");
    }
    
    Ok(())
}
