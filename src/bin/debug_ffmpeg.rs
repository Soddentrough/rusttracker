use ffmpeg_next as ffmpeg;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ffmpeg::init()?;
    
    let path = "/home/naoki/Downloads/Love.Death.and.Robots.S04E01.1080p.WEB.h264-ETHEL[EZTVx.to].mkv";
    println!("Opening file: {}", path);
    
    let mut ictx = ffmpeg::format::input(&path)?;
    
    let video_stream = ictx.streams().best(ffmpeg::media::Type::Video);
    let audio_stream = ictx.streams().best(ffmpeg::media::Type::Audio);
    
    println!("Video stream: {:?}", video_stream.as_ref().map(|s| s.index()));
    println!("Audio stream: {:?}", audio_stream.as_ref().map(|s| s.index()));
    
    let a_stream = audio_stream.expect("No audio stream found");
    let a_index = a_stream.index();
    let a_context = ffmpeg::codec::context::Context::from_parameters(a_stream.parameters())?;
    let mut a_decoder = a_context.decoder().audio()?;
    
    println!("Decoder layout: {:?}", a_decoder.channel_layout());
    println!("Decoder format: {:?}", a_decoder.format());
    println!("Decoder rate: {:?}", a_decoder.rate());
    
    // We try initializing the resampler
    let mut resampler = ffmpeg::software::resampling::context::Context::get(
        a_decoder.format(),
        a_decoder.channel_layout(),
        a_decoder.rate(),
        ffmpeg::format::sample::Sample::F32(ffmpeg::format::sample::Type::Packed),
        a_decoder.channel_layout(),
        a_decoder.rate(),
    )?;
    
    let v_stream = video_stream.expect("No video stream found");
    let v_index = v_stream.index();
    let v_context = ffmpeg::codec::context::Context::from_parameters(v_stream.parameters())?;
    let mut v_decoder = v_context.decoder().video()?;
    
    println!("Decoding loops starting...");
    
    let mut audio_packets_read = 0;
    let mut video_packets_read = 0;
    let mut audio_frames_decoded = 0;
    let mut video_frames_decoded = 0;
    
    let start = Instant::now();
    
    for (stream, packet) in ictx.packets() {
        if stream.index() == a_index {
            audio_packets_read += 1;
            println!("Read Audio Packet #{}, pts: {:?}", audio_packets_read, packet.pts());
            if let Err(e) = a_decoder.send_packet(&packet) {
                println!("Audio send_packet error: {:?}", e);
            }
            
            let mut decoded = ffmpeg::frame::Audio::empty();
            match a_decoder.receive_frame(&mut decoded) {
                Ok(_) => {
                    audio_frames_decoded += 1;
                    println!(" -> Decoded Audio Frame #{}, samples: {}, rate: {}, layout: {:?}", audio_frames_decoded, decoded.samples(), decoded.rate(), decoded.channel_layout());
                    
                    let mut resampled = ffmpeg::frame::Audio::empty();
                    match resampler.run(&decoded, &mut resampled) {
                        Ok(_) => {
                            println!("    -> Resampled successfully: samples: {}, channels: {}", resampled.samples(), resampled.channels());
                        }
                        Err(e) => {
                            println!("    -> Resampler run error: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    println!(" -> Audio receive_frame returned: {:?}", e);
                }
            }
        } else if stream.index() == v_index {
            video_packets_read += 1;
            println!("Read Video Packet #{}, pts: {:?}", video_packets_read, packet.pts());
            if let Err(e) = v_decoder.send_packet(&packet) {
                println!("Video send_packet error: {:?}", e);
            }
            
            let mut decoded = ffmpeg::frame::Video::empty();
            match v_decoder.receive_frame(&mut decoded) {
                Ok(_) => {
                    video_frames_decoded += 1;
                    println!(" -> Decoded Video Frame #{}, format: {:?}", video_frames_decoded, decoded.format());
                }
                Err(e) => {
                    println!(" -> Video receive_frame returned: {:?}", e);
                }
            }
        }
        
        if audio_frames_decoded >= 5 && video_frames_decoded >= 5 {
            break;
        }
        
        if start.elapsed().as_secs() > 10 {
            println!("Timeout reached!");
            break;
        }
    }
    
    println!("Done. Audio read: {}, decoded: {}. Video read: {}, decoded: {}", audio_packets_read, audio_frames_decoded, video_packets_read, video_frames_decoded);
    
    Ok(())
}
