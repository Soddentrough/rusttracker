# RustTracker 🎵🦀

![RustTracker UI](assets/screenshot_spectrum.png)

A high-performance, real-time audio visualizer, video player, and tracker module player built in Rust with Vulkan/WGPU hardware acceleration.

RustTracker leverages a **3-Thread DSP Architecture** and **Hardware-Accelerated Compute Shaders** to deliver ultra-low-latency playback and multi-channel spatial audio analysis at **2,000+ FPS**.

---

## 🚀 Features

* **24+ Advanced Hardware Visualizers:** Cinematic WGPU compute and fragment shaders mapped tightly to acoustic frequencies (Ferrofluid, 3D Fire, 3D Synthwave Racer, 3D VU Meter Rack, 3D Glass Water Lyrics, CRT Oscilloscope, Bioluminescence, Neon Corridor, Matrix Rain, and more).
* **Multi-Platform Support:** Native support for **Linux (Fedora/RHEL & Ubuntu/Debian)**, **Android (Mobile & Tablet)**, **Windows 10/11**, **macOS**, and **Steam Deck (Gaming Mode)**.
* **Universal Media Playback:**
  * **Tracker Modules:** `.MOD`, `.S3M`, `.IT`, `.XM` with real-time piano-roll pattern editor and synchronized channel VU meters.
  * **Lossless & Compressed Audio:** FLAC, WAV, MP3, AAC, OGG, Opus, AIFF.
  * **Video Playback & Multi-Track Audio:** MP4, MKV, AVI, WebM with hardware-accelerated video decoding, HDR color spaces, and instant switching between multiple audio tracks (e.g., Vocals, Instrumental, Stems, Dubs).
  * **Bitstream Passthrough:** Real-time SPDIF/HDMI bitstream passthrough for Dolby Atmos, DTS-HD, and TrueHD.
  * **Live Microphone Input:** Instant audio-reactive capture via `--mic`.
* **Synchronized Lyrics & Heatmap:** Embedded and LRC synchronized lyrics with real-time heatmaps and 3D vector TrueType glyph extrusion.
* **Intuitive Touch & Gamepad Controls:** Tailored touch gestures for mobile screens and native controller mapping for Steam Deck / Gamepads.

---

## 🎨 Visualizations

| Photorealistic Ferrofluid | Multi-Channel 3D Fire |
| :---: | :---: |
| ![Ferrofluid](assets/screenshot_ferrofluid.png) | ![3D Fire](assets/screenshot_3dfire.png) |

| CRT Oscilloscope | Frequency Spectrum |
| :---: | :---: |
| ![CRT](assets/screenshot_crt.png) | ![Spectrum](assets/screenshot_spectrum.png) |

---

## 📱 Mobile Gestures (Android)

RustTracker features a specialized dual-pane interface optimized for mobile and tablet displays:

### Portrait Mode (Two-Pane Layout)
* **Top Pane (Upper 50% — HUD / Track / Video):**
  * **Swipe Left / Right:** Cycle HUD tabs (`Channels` ➔ `Heatmap / Lyrics` ➔ `Track Info` ➔ `Video Stream`).
  * **Swipe Up:** Expand to **Fullscreen Video** *(when video stream is active)*.
  * **Swipe Down:** Exit Fullscreen Video back to 50/50 split view.
* **Bottom Pane (Lower 50% — Real-Time Visualizer):**
  * **Swipe Left / Right:** Switch to next / previous visualizer module.
* **Bottom Scrubber Edge (Bottom 12%):**
  * **Touch & Drag:** Real-time interactive timeline seeking across track duration.

### Landscape Mode (Full-Screen Visualizer)
* **Swipe Left / Right:** Switch visualizers.
* **Swipe Up / Down:** Rotate HUD overlays (`Channels`, `Heatmap / Lyrics`, `Track Info`, `Video`).

### Universal Gestures (Both Orientations)
* **Single Tap:** **Play / Pause** toggle *(or tap top-right `[📂 OPEN]` for system file picker)*.
* **Double Tap:** Toggle **HUD Visibility** (`HUD: Visible` / `HUD: Hidden`).
* **Two-Finger Tap:** **Cycle Audio Track** on multi-track media files.

---

## 🎮 Desktop & Gamepad Controls

| Action | Desktop Key | Gamepad (Xbox / Steam Deck) |
| :--- | :--- | :--- |
| **Play / Pause** | `Space` | `A` / `South Button` |
| **Seek Forward / Backward** | `Right` / `Left` Arrow | `D-Pad Right` / `D-Pad Left` |
| **Volume Up / Down** | `Up` / `Down` Arrow | `D-Pad Up` / `D-Pad Down` |
| **Next / Previous Visualizer** | `]` / `[` or `Page Down` / `Page Up` | `Right Bumper (RB)` / `Left Bumper (LB)` |
| **Cycle HUD Tabs** | `1`, `2`, `3`, `4` | `Y` / `North Button` |
| **Cycle Audio Track** | `T` | `X` / `West Button` |
| **Toggle HUD Visibility** | `Tab` | `Right Stick Click (R3)` |
| **Toggle Performance Stats** | `S` | `Back / View Button` |
| **Toggle Video Layout Mode** | `V` | `Left Stick Click (L3)` |
| **Toggle Mute** | `M` | `Left Trigger (LT)` |
| **Fullscreen Toggle** | `F` or `Alt + Enter` | — |

---

## ⚙️ Environment Variables & Tuning

* **`RUSTTRACKER_FPS_LIMIT=<fps>`**: Set a software framerate cap (e.g. `RUSTTRACKER_FPS_LIMIT=120` for power saving on laptops/handhelds). Defaults to uncapped hardware rate (2,000+ FPS).
* **`RUSTTRACKER_PROFILE=1`**: Enables GPU hardware timestamp profiling (`wgpu::Features::TIMESTAMP_QUERY`) in the Stats overlay (`S` key) to measure exact per-shader microsecond timings.

---

## 📦 Installation & Download

Pre-compiled packages for all platforms are available on the [GitHub Releases](https://github.com/Soddentrough/rusttracker/releases) page:

* **Android:** Download and install `app-release.apk` (or `RustTracker-Android-*.apk`).
* **Windows:** Download and run `RustTracker-Setup.exe` installer.
* **macOS:** Download and open `RustTracker-MacOS-Installer.dmg`.
* **Fedora / RHEL:** `sudo dnf install ./rusttracker-*.x86_64.rpm`
* **Ubuntu / Debian:** `sudo dpkg -i ./rusttracker_*_amd64.deb`
* **Steam Deck:** Download `RustTracker-SteamDeck-*.AppImage`, mark executable, and add as Non-Steam Game in Steam Gaming Mode.

---

## 🛠️ Building from Source

### Prerequisites
* Rust 1.85+ (Edition 2024)
* Vulkan SDK / Drivers (`vulkan-loader`, `mesa-vulkan-drivers`)
* FFmpeg 6.0+ libraries & `libopenmpt`

### Linux (Fedora)
```bash
sudo dnf install -y gcc gcc-c++ binutils pkgconfig alsa-lib-devel wayland-devel libX11-devel libxkbcommon-devel systemd-devel ffmpeg-devel libopenmpt-devel clang clang-devel
cargo build --release
```

### Android (APK)
```bash
# Requires Android SDK + NDK and cargo-ndk
cargo install cargo-ndk
rustup target add aarch64-linux-android
cd android && ./gradlew assembleRelease
```

---

## 📜 License

This project is licensed under the [GNU General Public License v3.0](LICENSE) (GPLv3).

