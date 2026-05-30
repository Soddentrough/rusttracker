#!/usr/bin/env python3
import os
import sys
import subprocess
import json
import argparse
from PIL import Image

VISUALIZERS = [
    "spectrum", "oscilloscope", "3doscilloscope", "3doscilloscope_freq",
    "flame", "firesim", "solar", "spatial", "ferrofluid", "ferrofluidsim",
    "neon", "lissajous", "synthwave", "starfield", "rain", "3drain",
    "synthwaveracer", "cuboids"
]

import math
import struct
import wave

def generate_multichannel_sweep(filename="test_multichannel_sweep.wav", duration=5.0, sample_rate=44100):
    if os.path.exists(filename):
        return
    print(f"Generating {filename} (5.1 surround sweep for testing)...")
    num_channels = 6
    num_samples = int(duration * sample_rate)
    
    with wave.open(filename, 'wb') as wav:
        wav.setnchannels(num_channels)
        wav.setsampwidth(2) # 16-bit
        wav.setframerate(sample_rate)
        
        frames = []
        for n in range(num_samples):
            t = n / sample_rate
            
            # Left (0): 100Hz to 1000Hz sweep
            f_l = 100.0 + (900.0 * (t / duration))
            val_l = math.sin(2.0 * math.pi * f_l * t)
            
            # Right (1): 1000Hz to 100Hz sweep
            f_r = 1000.0 - (900.0 * (t / duration))
            val_r = math.sin(2.0 * math.pi * f_r * t)
            
            # Center (2): 200Hz to 2000Hz sweep
            f_c = 200.0 + (1800.0 * (t / duration))
            val_c = math.sin(2.0 * math.pi * f_c * t)
            
            # LFE (3): 20Hz to 120Hz sweep (bass)
            f_lfe = 20.0 + (100.0 * (t / duration))
            val_lfe = math.sin(2.0 * math.pi * f_lfe * t)
            
            # Ls (4): 500Hz to 5000Hz sweep
            f_ls = 500.0 + (4500.0 * (t / duration))
            val_ls = math.sin(2.0 * math.pi * f_ls * t)
            
            # Rs (5): 5000Hz to 500Hz sweep
            f_rs = 5000.0 - (4500.0 * (t / duration))
            val_rs = math.sin(2.0 * math.pi * f_rs * t)
            
            l_int = int(val_l * 32767.0 * 0.5)
            r_int = int(val_r * 32767.0 * 0.5)
            c_int = int(val_c * 32767.0 * 0.5)
            lfe_int = int(val_lfe * 32767.0 * 0.8)
            ls_int = int(val_ls * 32767.0 * 0.5)
            rs_int = int(val_rs * 32767.0 * 0.5)
            
            frame = struct.pack('<hhhhhh', l_int, r_int, c_int, lfe_int, ls_int, rs_int)
            frames.append(frame)
            
        wav.writeframes(b''.join(frames))
    print(f"Successfully generated {filename}")


def run_benchmark(vis_name, mode, frames, use_release):
    print(f"Running benchmark for '{vis_name}' ({mode} mode)...")
    
    # Configure directories and paths
    os.makedirs("test_baselines", exist_ok=True)
    capture_path = os.path.abspath(f"test_baselines/{vis_name}_{mode}.png")
    
    # Set env vars
    env = os.environ.copy()
    env["CAPTURE_FRAME"] = "1"
    env["CAPTURE_PATH"] = capture_path
    
    # Build cargo run command
    cargo_cmd = ["cargo", "run"]
    if use_release:
        cargo_cmd.append("--release")
    cargo_cmd.extend(["--", "--vis", vis_name, "--bench", str(frames)])
    
    # Use test_multichannel_sweep.wav as reference input if available, else test_sine.wav
    if os.path.exists("test_multichannel_sweep.wav"):
        cargo_cmd.append("test_multichannel_sweep.wav")
    elif os.path.exists("test_sine.wav"):
        cargo_cmd.append("test_sine.wav")


    
    try:
        # Run process and capture stdout
        result = subprocess.run(
            cargo_cmd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=True
        )
        
        # Parse output metrics
        fps = 0.0
        shader_us = 0.0
        render_us = 0.0
        
        for line in result.stdout.splitlines():
            if line.startswith("BENCHMARK_RESULT_FPS:"):
                fps = float(line.split(":")[1].strip())
            elif line.startswith("BENCHMARK_RESULT_SHADER_US:"):
                shader_us = float(line.split(":")[1].strip())
            elif line.startswith("BENCHMARK_RESULT_RENDER_US:"):
                render_us = float(line.split(":")[1].strip())
                
        metrics = {
            "fps": fps,
            "shader_us": shader_us,
            "render_us": render_us,
            "screenshot": capture_path
        }
        
        print(f"  FPS: {fps:.2f} | Shader Time: {shader_us:.2f} µs | Render Time: {render_us:.2f} µs")
        return metrics
    except subprocess.CalledProcessError as e:
        print(f"Error running benchmark for {vis_name}:")
        print(e.stderr)
        return None

def compare_screenshots(base_path, test_path):
    if not os.path.exists(base_path) or not os.path.exists(test_path):
        return 1.0, "Missing one or both screenshots for comparison"
    
    try:
        img_base = Image.open(base_path).convert("RGB")
        img_test = Image.open(test_path).convert("RGB")
        
        if img_base.size != img_test.size:
            return 1.0, f"Dimensions mismatch: {img_base.size} vs {img_test.size}"
        
        pixels_base = list(img_base.getdata())
        pixels_test = list(img_test.getdata())
        total_pixels = len(pixels_base)
        
        mse = 0.0
        for p1, p2 in zip(pixels_base, pixels_test):
            mse += (p1[0] - p2[0]) ** 2 + (p1[1] - p2[1]) ** 2 + (p1[2] - p2[2]) ** 2
        
        mse /= (total_pixels * 3)
        normalized_diff = mse / (255.0 ** 2)
        
        return normalized_diff, None
    except Exception as e:
        return 1.0, f"Error comparing images: {str(e)}"

def main():
    generate_multichannel_sweep()
    parser = argparse.ArgumentParser(description="RustTracker Visual and Performance Regression Test Harness")
    parser.add_argument("--mode", choices=["baseline", "test", "compare"], required=True,
                        help="Mode: 'baseline' to record references, 'test' to record current build, 'compare' to analyze differences.")
    parser.add_argument("--frames", type=int, default=300, help="Number of frames to run (default: 300 = 5 seconds @ 60fps)")
    parser.add_argument("--debug", action="store_true", help="Run in debug mode (default is release mode)")
    parser.add_argument("--vis", help="Run only for a specific visualization (comma-separated list, or single name)")
    
    args = parser.parse_args()
    use_release = not args.debug
    
    selected_vis = VISUALIZERS
    if args.vis:
        selected_vis = [v.strip() for v in args.vis.split(",") if v.strip() in VISUALIZERS]
        if not selected_vis:
            print(f"Error: None of the specified visualizers match the available set.")
            sys.exit(1)
            
    metrics_file = f"test_baselines/{args.mode}_metrics.json"
    
    if args.mode in ["baseline", "test"]:
        results = {}
        for vis in selected_vis:
            metrics = run_benchmark(vis, args.mode, args.frames, use_release)
            if metrics:
                results[vis] = metrics
                
        with open(metrics_file, "w") as f:
            json.dump(results, f, indent=4)
        print(f"\n{args.mode.capitalize()} run complete. Metrics written to {metrics_file}")
        
    elif args.mode == "compare":
        base_file = "test_baselines/baseline_metrics.json"
        test_file = "test_baselines/test_metrics.json"
        
        if not os.path.exists(base_file) or not os.path.exists(test_file):
            print(f"Error: Both '{base_file}' and '{test_file}' must exist to run a comparison.")
            print("Please run with --mode baseline and --mode test first.")
            sys.exit(1)
            
        with open(base_file, "r") as f:
            base_data = json.load(f)
        with open(test_file, "r") as f:
            test_data = json.load(f)
            
        print("\n=======================================================")
        print("                 REGRESSION COMPARISON RESULTS         ")
        print("=======================================================")
        
        markdown_lines = [
            "# Regression Comparison Report",
            "",
            "This report compares the performance and visuals of the updated shaders against the baseline.",
            "",
            "| Visualizer | Baseline FPS | Test FPS | FPS Change | Baseline Shader (µs) | Test Shader (µs) | Shader Change | Visual Mismatch (MSE) |",
            "| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |"
        ]
        
        any_perf_degraded = False
        any_visual_mismatch = False
        
        for vis in selected_vis:
            if vis not in base_data or vis not in test_data:
                print(f"Skipping {vis}: missing data in baseline or test.")
                continue
                
            base_m = base_data[vis]
            test_m = test_data[vis]
            
            fps_pct = ((test_m["fps"] - base_m["fps"]) / base_m["fps"]) * 100.0 if base_m["fps"] > 0 else 0.0
            shader_pct = ((test_m["shader_us"] - base_m["shader_us"]) / base_m["shader_us"]) * 100.0 if base_m["shader_us"] > 0 else 0.0
            
            diff_val, err = compare_screenshots(base_m["screenshot"], test_m["screenshot"])
            
            # Format differences
            fps_change_str = f"{fps_pct:+.1f}%"
            shader_change_str = f"{shader_pct:+.1f}%"
            
            # Color/alert highlights for markdown
            if fps_pct < -5.0:
                fps_change_str = f"**{fps_change_str}** ⚠️"
                any_perf_degraded = True
            if shader_pct > 10.0:
                shader_change_str = f"**{shader_change_str}** ⚠️"
                any_perf_degraded = True
                
            visual_status = "0.0% (Identical)"
            if err:
                visual_status = f"Err: {err}"
            else:
                mismatch_pct = diff_val * 100.0
                # Note: noise/particles cause slight changes. High threshold > 1.5% might suggest mismatch
                if mismatch_pct > 1.5:
                    visual_status = f"**{mismatch_pct:.3f}%** ⚠️"
                    any_visual_mismatch = True
                else:
                    visual_status = f"{mismatch_pct:.3f}%"
            
            markdown_lines.append(
                f"| `{vis}` | {base_m['fps']:.1f} | {test_m['fps']:.1f} | {fps_change_str} | {base_m['shader_us']:.1f} | {test_m['shader_us']:.1f} | {shader_change_str} | {visual_status} |"
            )
            
            # Print to stdout
            print(f"\nVisualizer: {vis}")
            print(f"  FPS: {base_m['fps']:.1f} -> {test_m['fps']:.1f} ({fps_pct:+.1f}%)")
            print(f"  Shader: {base_m['shader_us']:.1f} µs -> {test_m['shader_us']:.1f} µs ({shader_pct:+.1f}%)")
            if err:
                print(f"  Visuals: Comparison error - {err}")
            else:
                print(f"  Visual Mismatch: {diff_val*100.0:.3f}%")
                
        # Append warnings to report
        markdown_lines.append("")
        if any_perf_degraded:
            markdown_lines.append("> [!WARNING]")
            markdown_lines.append("> Performance regressions of >5% FPS drop or >10% shader time increase were detected (marked with ⚠️). Please check if shader loops or allocations can be optimized.")
            markdown_lines.append("")
        if any_visual_mismatch:
            markdown_lines.append("> [!NOTE]")
            markdown_lines.append("> Visual mismatches of >1.5% (marked with ⚠️) may indicate changes in rendering paths or layout. Note that minor mismatches are normal for shaders using time-dependent or random/procedural elements.")
            markdown_lines.append("")
            
        report_path = "test_baselines/regression_report.md"
        with open(report_path, "w") as f:
            f.write("\n".join(markdown_lines))
            
        print(f"\nComparison report written to {report_path}")

if __name__ == "__main__":
    main()
