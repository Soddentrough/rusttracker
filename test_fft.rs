use spectrum_analyzer::{samples_fft_to_spectrum, scaling::divide_by_N_sqrt, windows::hann_window, FrequencyLimit};
use std::time::Instant;

fn main() {
    let mut buffer = vec![0.0f32; 512];
    for i in 0..512 { buffer[i] = (i as f32).sin(); }
    let start = Instant::now();
    for _ in 0..1000 {
        let hann = hann_window(&buffer);
        let _ = samples_fft_to_spectrum(&hann, 48000, FrequencyLimit::Max(10000.0), Some(&divide_by_N_sqrt));
    }
    println!("1000 FFTs took {:?}", start.elapsed());
}
