import re

with open("src/bitstream.rs", "r") as f:
    content = f.read()

# Make it a module without the `main` fn.
content = content.replace("fn main() -> Result<()> {", "fn __removed() {")

# Expose DspMessage
content = """#[cfg(target_os = "windows")]
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

""" + content

content = content.replace("pub fn run(file_path: &str, device_idx: Option<u32>) -> Result<()> {", """pub fn start_bitstream_thread(file_path: &str, shared_state: Arc<Mutex<AppState>>, tx: Sender<DspMessage>) -> Result<std::thread::JoinHandle<()>> {""")

content = content.replace("let device = if let Some(idx) = device_idx {", "let device_idx: Option<u32> = None; let device = if let Some(idx) = device_idx {")

# Replace ffmpeg thread spawn to include decoder
decoder_logic = """
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
            let mut decoder = ictx.streams().best(ffmpeg_next::media::Type::Audio).unwrap().codec().decoder().audio().unwrap();
            let mut resampler = ffmpeg_next::software::resampling::context::Context::get(
                decoder.format(), decoder.channel_layout(), decoder.rate(),
                ffmpeg_next::format::sample::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
                decoder.channel_layout(), decoder.rate(),
            ).unwrap();

            let mut pkts_written = 0;
            for (stream, mut packet) in ictx.packets() {
                if stream.index() == best_audio_index {
                    let vis_packet = packet.clone();

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
                                let mut channel_audio_data = Vec::with_capacity(planes);
                                let mut channel_vus = Vec::with_capacity(planes);
                                for p in 0..planes {
                                    let data = resampled.plane::<f32>(p);
                                    channel_audio_data.push(data.to_vec());
                                    
                                    // Calculate simple RMS for VU
                                    let mut sum_sq = 0.0;
                                    for &s in data { sum_sq += s * s; }
                                    let rms = (sum_sq / data.len() as f32).sqrt();
                                    channel_vus.push(rms);
                                }
                                let _ = tx.send(DspMessage {
                                    audio_data: channel_audio_data[0].clone(),
                                    channel_vus,
                                    current_order: 0,
                                    current_row: 0,
                                    bpm: 0,
                                    speed: 0,
                                    current_seconds: 0.0,
                                    current_row_string: "".to_string(),
                                    channel_audio_data,
                                });
                            }
                        }
                    }
                }
            }
            let _ = octx.write_trailer();
        });
"""

# Use regex to replace the entire ffmpeg_thread block
content = re.sub(r'let ffmpeg_thread = std::thread::spawn\(move \|\| \{.*?\n        \}\);', decoder_logic.strip(), content, flags=re.DOTALL)

# Modify the end to return the JoinHandle instead of Ok(())
end_logic = """
        let handle = std::thread::spawn(move || {
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
"""

content = re.sub(r'loop \{.*Ok\(\(\)\)\n    \}', end_logic.strip() + "\n    }", content, flags=re.DOTALL)

with open("src/bitstream.rs", "w") as f:
    f.write(content)

