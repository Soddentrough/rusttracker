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

## [v0.9.4 Target] Architectural Migration: Native 3D Visualization Pipeline

**Feature Request:**
Extend the existing `wgpu` rendering architecture in `engine.rs` to support a traditional 3D graphics pipeline (vertex buffers, meshes, depth testing, MVP matrices) while fully retaining compatibility with the existing Shadertoy-style (full-screen quad) 2D visualizers.

**What Needs to Change:**
*   **Pipeline Descriptors:** Modify `VulkanEngine` to support multiple pipeline configurations. Currently, the `RenderPipelineDescriptor` is hardcoded to `depth_stencil: None` and `buffers: &[]` (no vertex attributes).
*   **Vertex Data & Buffers:** Introduce a system to define, allocate, and bind `wgpu::Buffer` objects for vertices, indices, normals, and UVs.
*   **Depth Texture:** Allocate a Z-buffer (depth texture, e.g., `TextureFormat::Depth32Float`) and bind it to the render pass descriptor to ensure proper 3D occlusion.
*   **Camera & Transform Uniforms:** Create a new uniform buffer for Model-View-Projection (MVP) matrices to allow for camera movement and object transformations.
*   **Render Loop Refactor:** Update the main render loop to dynamically switch between drawing a single full-screen quad (for legacy shaders) and issuing multiple `draw_indexed` calls (for 3D meshes).

**How Existing Functionality Can Be Protected:**
*   **Visualizer Trait/Enum Abstraction:** Abstract the visualizers behind an enum (e.g., `VisualizerType::FragmentOnly` vs `VisualizerType::Native3D`) in `state.rs`. 
*   **Pipeline Segregation:** Keep the existing `render_pipelines` exactly as they are. Create a new, separate set of `wgpu::RenderPipeline` objects specifically configured for 3D rendering (with depth testing enabled and vertex layouts defined).
*   **Render Pass Isolation:** During the frame render, the engine checks the active visualizer type. If it's a legacy shader, it uses the existing pipeline and ignores the depth attachment. If it's 3D, it binds the 3D pipeline, attaches the depth buffer, and binds vertex buffers.
*   **Shared Audio Context:** Both pipeline types will continue to bind the exact same `AudioUniforms` bind group, meaning the audio analysis logic and bindings remain identical and robust across both paradigms.

**What Enhancements This Shift Would Allow:**
*   **True 3D Assets:** Loading standard 3D models (e.g., `.gltf`, `.obj`) directly into visualizers without relying on complex and expensive raymarching math (This ties directly into the "Neon Room" `blend` feature request).
*   **Complex Scene Graphs:** Rendering scenes with independent moving objects, rigid body physics, and particle systems.
*   **Lower GPU Overhead for 3D:** Traditional rasterization is significantly less computationally intensive than raymarching per-pixel for complex geometry, allowing high-fidelity 3D scenes to run at 60+ FPS on lower-end hardware (like the Steam Deck).
*   **Advanced Interactive Cameras:** Real-time fly-throughs, orbital cameras, and perspective shifts controlled by gamepad or mouse, making visualizers far more interactive.

**Pilot Implementation: Neon Room Visualizer**
Once the native 3D pipeline is established, the first 3D visualizer will be based on `assets/neon_room.blend`.
*   **Concept:** A room scene with 3D objects labelled after spatial channels (Front, LFE, center, rear, etc). The 3D engine will light up these objects dynamically in time to the audio streams.
*   **Asset Pipeline:** Requires integrating a loader (e.g., the `gltf` or `blend-rs` crate) to import the 3D object data and material properties into our new `wgpu` vertex buffers.
*   **Lighting Strategy:** Instead of full hardware raytracing (which is too computationally intense for 120Hz on lower-end hardware), we will use standard 3D rasterization with dynamic point/spot lights linked to the `AudioUniforms` data.
*   **Remaining Concerns:** Binary file size bloat from embedding 3D assets will need to be actively managed by optimizing model exports.

## Future Target: Vaporwave Fountain Visualizer

**Feature Request:**
Implement a Vaporwave Fountain visualizer.
*   **Concept:** A retro 3D fountain utilizing instanced particles for water droplets, with water jets that spray and change height/angle dynamically in reaction to audio frequencies.
*   **Styling:** A classic Black & White (B&W) film tone.
*   **Concerns:** Achieving an authentic B&W film look (film grain, bloom, vignette, proper contrast curves) alongside high-performance instanced particle updates on the GPU can be challenging and will require careful optimization.
* We should not use hacky quasi-3D fragment shaders when dealing with 3D. No PipelineType::FullscreenQuad. Only fully 3D Mesh3D scenes. We need to decide which visualizations need to be migrated or re-written. Also, which shouldn't be for performance reasons.
