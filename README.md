# RustTracker 🎵

![RustTracker UI](assets/screenshot_ferrofluid.png)

A high-performance, real-time audio visualizer and tracker module player built in Rust. 

RustTracker leverages a **3-Thread DSP Architecture** and **Hardware-Accelerated Compute Shaders** to ensure zero-latency audio playback while simultaneously computing **GPU-accelerated Fast Fourier Transforms (FFT) on each spatial audio channel** to render beautiful, fluid visualizations at 120FPS.

## Visualizations

RustTracker includes a variety of WGPU-accelerated visualizations, combining classic demoscene aesthetics with modern procedural generation.

| Photorealistic Ferrofluid | Multi-Channel 3D Fire |
| :---: | :---: |
| ![Ferrofluid](assets/screenshot_ferrofluid.png) | ![3D Fire](assets/screenshot_3dfire.png) |

| CRT Oscilloscope | Frequency Spectrum |
| :---: | :---: |
| ![CRT](assets/screenshot_crt.png) | ![Spectrum](assets/screenshot_spectrum.png) |

## Features

* **Advanced Visualizers:** High-fidelity, cinematic shaders mapped tightly to precise acoustic frequencies.
* **GPU-Accelerated FFT:** Offloads audio-reactive spatial weights directly to the GPU using WGPU compute shaders, mapping individual surround-sound speaker channels to local geometry in real-time.
* **Cinematic Video Integration & HDR:** Features integrated hardware-accelerated video stream playback, rendering vibrant visual environments with HDR color precision.
* **Real-time Tracker UI:** Seamlessly decodes and visualizes `.mod` files, rendering a classic piano-roll style pattern editor that aligns perfectly with the audio playback, complete with flawless cross-pattern scrolling and jumping.
* **Graceful Degradation:** Supports playing standard audio/video files (`.mp3`, `.flac`, `.mp4`, `.mkv`, etc.) or capturing live microphone input (`--mic`), instantly adapting the UI to remove tracker elements and focus on acoustic analysis.
* **Zero-Latency Architecture:**
  * **Audio Thread (`cpal`):** A lock-free, ultra-high-priority thread dedicated solely to IO, preventing stuttering and audio underruns.
  * **DSP Thread:** A background worker that computes windowing and frequency history data without blocking the audio stream.
  * **GPU Render Pipeline (`wgpu` + `egui`):** Low-overhead, high-performance hardware rendering.

## Quick Start

Run a tracker module:
```bash
cargo run -- path/to/your_file.mod
```

Run with live microphone input:
```bash
cargo run -- --mic
```

## Steam Deck Installation

RustTracker provides a fully optimized native AppImage specifically built for SteamOS, featuring out-of-the-box gamepad controls.

1. **Get the AppImage onto your Steam Deck:**
   - **Browser:** Open Firefox/Chrome on the Deck in Desktop Mode and download `RustTracker-SteamDeck-GamePad.AppImage` directly from the **Releases** page to your `Downloads` folder.
   - **Network (SSH/SCP):** If SSH is enabled on your Deck (`sudo systemctl start sshd`), copy it from your PC: `scp RustTracker-SteamDeck-GamePad.AppImage deck@<DECK_IP>:/home/deck/Downloads/`
   - **USB/SD Card:** Copy the file to a USB-C drive or SD card and transfer it via the Deck's file manager (Dolphin).
2. Switch your Steam Deck to **Desktop Mode** (if you aren't already).
3. Open your Downloads folder, right-click the AppImage file, and select **Properties** -> **Permissions**. Check the box for **"Is executable"**.
4. Right-click the file again and select **"Add to Steam"**.
5. Switch back to **Gaming Mode** and launch RustTracker from your Non-Steam games library.

**Important Note on Controls:** 
RustTracker expects native Gamepad inputs. Launching it via Gaming Mode ensures Steam Input sends proper gamepad signals (Y, X, A, B, etc.). If you run the AppImage from Desktop Mode without adding it to Steam, Steam Input defaults to its "Desktop Configuration" (which translates your button presses into keyboard keys like `Escape` and `Tab`), rendering the native Gamepad UI features unresponsive. 

To force Gamepad Mode while on the desktop, hold the **Start (Menu)** button for 3 seconds!

## Built With
* `wgpu` & `egui` - Hardware-accelerated UI
* `cpal` - Cross-platform Audio I/O
* `spectrum-analyzer` - Fast Fourier Transforms
* `crossbeam-channel` - Lock-free concurrency
* `openmpt` - Tracker module decoding
