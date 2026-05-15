use symphonia::core::formats::FormatOptions;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;
use std::fs::File;

#[test]
fn test_symphonia() {
    let file_path = "/home/naoki/Downloads/Love.Death.and.Robots.S04E01.1080p.WEB.h264-ETHEL[EZTVx.to].mkv";
    let file = File::open(file_path).unwrap();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mkv");

    let probed = match symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()) {
            Ok(p) => p,
            Err(e) => {
                println!("Symphonia probe failed: {:?}", e);
                return;
            }
        };

    let format = probed.format;
    
    let track = format.tracks().iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL && 
                  symphonia::default::get_codecs().make(&t.codec_params, &symphonia::core::codecs::DecoderOptions::default()).is_ok());
                  
    println!("Found track: {:?}", track.is_some());
}
