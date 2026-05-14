fn main() {
    let sr = cpal::SampleRate(44100);
    let config = cpal::StreamConfig { channels: 2, sample_rate: sr, buffer_size: cpal::BufferSize::Default };
    let x: u32 = config.sample_rate;
}
