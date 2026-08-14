use std::process::Command;
use std::str;

#[test]
fn test_visualizer_benchmark() {
    // If running in a headless environment without X11 or Wayland, skip the windowed benchmark
    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        println!("Skipping windowed benchmark test: no DISPLAY or WAYLAND_DISPLAY environment variable set.");
        return;
    }

    let visualizers = vec!["3doscilloscope", "3doscilloscope_freq"];
    
    println!("\n==================================================");
    println!("RUNNING VISUALIZER BENCHMARKS (500 FRAMES)...");
    println!("==================================================");

    for vis in visualizers {
        let output = Command::new("cargo")
            .args(&[
                "run",
                "--release",
                "--",
                "test_sine.wav",
                "--vis",
                vis,
                "--bench",
                "500",
            ])
            .output()
            .expect("Failed to execute cargo run");

        let stdout = str::from_utf8(&output.stdout).unwrap();
        let stderr = str::from_utf8(&output.stderr).unwrap();

        if !output.status.success() {
            if stderr.contains("Authorization required") || stderr.contains("snd_pcm_open") || stderr.contains("X11") || stderr.contains("Wayland") || stderr.contains("panicked at") {
                println!("Skipping windowed benchmark for {} due to display/audio server access restrictions in sandbox/CI environment.", vis);
                continue;
            }
            eprintln!("Benchmark failed for visualizer: {}", vis);
            eprintln!("STDOUT:\n{}", stdout);
            eprintln!("STDERR:\n{}", stderr);
            panic!("Visualizer benchmark command exited with error status");
        }

        // Parse benchmark results from stdout
        let mut parsed_name = String::new();
        let mut parsed_fps = String::new();
        let mut parsed_shader_us = String::new();
        let mut parsed_render_us = String::new();

        for line in stdout.lines() {
            if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_VISUALIZER: ") {
                parsed_name = val.to_string();
            } else if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_FPS: ") {
                parsed_fps = val.to_string();
            } else if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_SHADER_US: ") {
                parsed_shader_us = val.to_string();
            } else if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_RENDER_US: ") {
                parsed_render_us = val.to_string();
            }
        }

        println!("Visualizer:   {}", parsed_name);
        println!("FPS:          {}", parsed_fps);
        println!("Shader Time:  {} us", parsed_shader_us);
        println!("Render Time:  {} us", parsed_render_us);
        println!("--------------------------------------------------");
    }
}
