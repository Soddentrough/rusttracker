use std::process::Command;
use std::str;

#[test]
fn test_visualizer_benchmark() {
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
