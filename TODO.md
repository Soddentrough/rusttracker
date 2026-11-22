# RustTracker TODO List

## MacOS Bitstream/Passthrough Implementation

**Feature Request:**
Implement native bitstream passthrough for macOS to support spatial surround formats (AC3, E-AC3) directly via HDMI/Optical.

**Implementation Notes:**
*   macOS relies on CoreAudio. To support bitstreaming, we must write a HAL (Hardware Abstraction Layer) implementation.
*   Requires bypassing the macOS system mixer by seizing "Hog Mode" (`kAudioDevicePropertyHogMode`) on the audio device.
*   Must explicitly set the device's physical stream format to `kAudioFormat60958AC3` (or similar IEC 61937 sub-formats) to tell the OS to send raw frames.

**Concerns:**
*   Apple Silicon Macs are heavily restricted regarding HDMI audio passthrough. High-bandwidth lossless formats like TrueHD and DTS-HD Master Audio are likely to be stripped or decoded natively and may require EDID spoofers.

## Architectural Migration: Native 3D Visualization Pipeline *(Completed)*

Implemented in [`crates/rt-render/src/engine/`](crates/rt-render/src/engine/mod.rs) and [`crates/rt-core/src/state.rs`](crates/rt-core/src/state.rs):
*   `VisualizerDef::pipeline_type` cleanly segregates 2D fragment shaders (`PipelineType::FullscreenTriangle`, replacing legacy quad rendering) from native hardware-rasterized 3D scenes (`PipelineType::Mesh3D`).
*   Shared `Depth32Float` depth attachment, camera/projection uniforms, instanced unit geometry (`UnitBox`, `UnitQuad`), OBJ geometry, and procedural 3D meshes power the 3D visualizer suite (`3D Oscilloscope`, `Strings`, `Retro VU Meters`, `Bioluminescent Waves`, `Neon Acoustic Studio`, `3D Glass Water Lyrics`, and `Prismatic Spectrum Canyon`).

## Future Target: Vaporwave Fountain Visualizer

**Feature Request:**
Implement a Vaporwave Fountain visualizer.
*   **Concept:** A retro 3D fountain utilizing instanced particles for water droplets via `PipelineType::Mesh3D`, with water jets that spray and change height/angle dynamically in reaction to audio frequencies.
*   **Styling:** A classic Black & White (B&W) film tone.
*   **Concerns:** Achieving an authentic B&W film look (film grain, bloom, vignette, proper contrast curves) alongside high-performance instanced particle updates on the GPU will require careful optimization.

## Open Platform & Architecture Roadmap
*   **TOML Config & Shared Shader Constants:** Persist visualizer enable lists, device preferences, and presets in a config file, and generate shared WGSL/host constants from one source.
*   **Adaptive Render-Scale Tier:** Optional `0.5×–0.75×` internal render scaling for heavy compute/raymarch visualizers on handheld and mobile GPUs.
*   **Zero-Copy Hardware Video Import:** `VK_EXT_external_memory_dma_buf` / Vulkan Video path when exposed cleanly through `wgpu`.
