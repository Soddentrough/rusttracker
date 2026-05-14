use alsa::{Direction, ValueOr};
use alsa::pcm::{PCM, HwParams, Format, Access};
use anyhow::{Context, Result};
use std::env;
use std::io::Read;
use std::process::{Command, Stdio};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <file_path> <alsa_device>", args[0]);
        eprintln!("Example: {} movie.mkv 'hdmi:CARD=HDMI,DEV=0,AES0=0x02'", args[0]);
        return Ok(());
    }

    let file_path = &args[1];
    let device_name = &args[2];

    println!("Starting FFmpeg to probe encapsulated IEC61937 rate...");
    
    // We must know the sample rate the spdif muxer will output.
    // Usually, AC3 = 48000, TrueHD = 192000, DTS-HD = 192000.
    // We can just try 192000 and 48000 based on the codec, but FFmpeg is tricky.
    // Let's just use FFprobe on the spdif format directly!
    let probe_output = Command::new("ffprobe")
        .arg("-v").arg("error")
        .arg("-select_streams").arg("a:0")
        .arg("-show_entries").arg("stream=sample_rate,codec_name")
        .arg("-of").arg("default=noprint_wrappers=1:nokey=1")
        .arg("-f").arg("spdif")
        .arg(file_path)
        .output()
        .context("Failed to run ffprobe. Is ffmpeg installed?")?;
        
    let probe_str = String::from_utf8_lossy(&probe_output.stdout);
    let mut lines = probe_str.trim().lines();
    let codec_name = lines.next().unwrap_or("unknown");
    let base_rate: u32 = lines.next().unwrap_or("48000").parse().unwrap_or(48000);
    
    // IEC61937 formats:
    // AC3/DTS use 48kHz
    // TrueHD/DTS-HD MA use 192kHz or HBR (8ch 192kHz).
    // The spdif muxer outputs 2 channels of s16le.
    // For TrueHD, it outputs 2ch 192000Hz.
    let rate = if codec_name.contains("truehd") || codec_name.contains("dts") || codec_name.contains("dca") {
        192000
    } else {
        base_rate
    };

    println!("Detected codec: {}, target IEC61937 rate: {} Hz", codec_name, rate);
    println!("Opening ALSA device: {}", device_name);

    let pcm = PCM::new(device_name, Direction::Playback, false)
        .context("Failed to open ALSA device")?;

    {
        let hwp = HwParams::any(&pcm)?;
        hwp.set_channels(2)?;
        hwp.set_rate(rate, ValueOr::Nearest)?;
        hwp.set_format(Format::s16())?;
        hwp.set_access(Access::RWInterleaved)?;
        pcm.hw_params(&hwp).context("Failed to set ALSA HW parameters")?;
    }
    
    let io = pcm.io_i16()?;

    println!("Spawning FFmpeg to mux to SPDIF...");
    let mut child = Command::new("ffmpeg")
        .arg("-v").arg("error")
        .arg("-i").arg(file_path)
        .arg("-c:a").arg("copy")
        .arg("-f").arg("spdif")
        .arg("-")
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn ffmpeg")?;

    let mut stdout = child.stdout.take().unwrap();
    let mut buffer = [0u8; 8192];
    
    println!("Bitstreaming started. Press Ctrl+C to stop.");
    
    loop {
        let bytes_read = stdout.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        
        // Convert u8 buffer to i16 slice for ALSA
        let frames = bytes_read / 4; // 2 bytes per sample * 2 channels = 4 bytes per frame
        if frames > 0 {
            let i16_slice: &[i16] = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr() as *const i16, frames * 2)
            };
            
            match io.writei(i16_slice) {
                Ok(_) => {},
                Err(err) => {
                    pcm.try_recover(err, false)?;
                }
            }
        }
    }

    let _ = child.wait();
    println!("Playback finished.");
    Ok(())
}
