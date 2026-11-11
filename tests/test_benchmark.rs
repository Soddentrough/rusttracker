use std::process::Command;
use std::str;

#[test]
fn test_visualizer_benchmark() {
    // If running in a headless environment without X11 or Wayland, skip the windowed benchmark
    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        println!(
            "Skipping windowed benchmark test: no DISPLAY or WAYLAND_DISPLAY environment variable set."
        );
        return;
    }

    let visualizers = vec!["spectrum", "glass_water"];

    println!("\n==================================================");
    println!("RUNNING VISUALIZER BENCHMARKS (60 FRAMES)...");
    println!("==================================================");

    for vis in visualizers {
        let output = Command::new(env!("CARGO_BIN_EXE_rusttracker"))
            .args(["test_sine.wav", "--windowed", "--vis", vis, "--bench", "60"])
            .output()
            .expect("Failed to execute rusttracker benchmark binary");

        let stdout = str::from_utf8(&output.stdout).unwrap();
        let stderr = str::from_utf8(&output.stderr).unwrap();

        if !output.status.success() {
            if stderr.contains("Authorization required")
                || stderr.contains("snd_pcm_open")
                || stderr.contains("Failed to initialize EventLoop")
                || stderr.contains("Failed to find an appropriate graphic adapter")
            {
                println!(
                    "Skipping windowed benchmark for {} due to display/audio server access restrictions in sandbox/CI environment.",
                    vis
                );
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
                parsed_name = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_FPS: ") {
                parsed_fps = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_SHADER_US: ") {
                parsed_shader_us = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("BENCHMARK_RESULT_RENDER_US: ") {
                parsed_render_us = val.trim().to_string();
            }
        }

        assert!(
            !parsed_name.is_empty(),
            "Benchmark output missing BENCHMARK_RESULT_VISUALIZER for {vis}\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
        );
        let fps_val: f32 = parsed_fps.parse().unwrap_or_else(|e| {
            panic!("Invalid BENCHMARK_RESULT_FPS '{parsed_fps}' for {vis}: {e}\nSTDOUT:\n{stdout}")
        });
        assert!(
            fps_val > 0.0,
            "Expected positive FPS for {vis}, got {fps_val}"
        );

        println!("Visualizer:   {}", parsed_name);
        println!("FPS:          {}", parsed_fps);
        println!("Shader Time:  {} us", parsed_shader_us);
        println!("Render Time:  {} us", parsed_render_us);
        println!("--------------------------------------------------");
    }
}
