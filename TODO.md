# RustTracker TODO List

## Version 0.8: Video Stream Integration

**Feature Request:** 
Support playing the background video stream from loaded media files (like MP4, MKV) directly in the "Track Info" UI panel, with an optional 'v' toggle (default on if supported, 'v' cycles through modes: Inside square 'Track Info' panel, takes over the entire top half replacing Channels, Heatmap, and Track Info, and full screen mode).

**Implementation Notes:**
*   **Zero-CPU Architecture:** CPU-based decoding and YUV->RGB scaling is strictly prohibited to protect the 120Hz/2.0ms frame time budget. All work must be offloaded to the GPU.
*   **Hardware Acceleration:** 
    *   Leverage hardware video decoders (e.g., NVDEC, VAAPI, or Vulkan Video) to decode the video stream.
    *   This may require evaluating if `ffmpeg_next` can properly expose hardware decode surfaces without unsafe FFI hell, or investigating `gstreamer-rs` which provides robust zero-copy hardware decode pipelines.
*   **GPU Color Conversion:** If the hardware decoder outputs YUV frames, the conversion to RGB must occur natively on the GPU (e.g., uploading the raw Y, U, and V planes as textures and converting them to RGB inside a WGSL shader) rather than using CPU `libswscale`.
*   **Threading Separation:** 
    *   The demuxing process must be refactored. A background thread will read raw packets from the container and push them to separate Audio and Video ring buffers.
    *   This ensures the high-priority CPAL audio thread is never blocked by video packet fetching or decoding.
*   **Difficulty Adjustment:** By shifting the decoding and scaling from CPU to GPU hardware, the performance impact on the main application is minimized, significantly lowering the risk of audio underruns or frame drops, making the feature highly viable.

## Version 0.9: Neon Room Ray Traced Visualizer

**Feature Request:**
Load `assets/neon_room.blend` to use as the basis for a new visualizer. It is a basic room scene with objects labelled after spatial channels (Front, LFE, center, rear, etc). 

The idea will be to load this scene and use a ray tracing engine to light up these objects in time to the audio streams.

**Implementation Notes:**
*   This may require the 'blend-rs' crate or some other system to load the 3d object data. It may require changes to the materials in Blender.

**Concerns:**
*   Binary file size bloat.
*   Computational intensity of a ray traced scene with dynamic lighting. Obviously only available on supported hardware. Must render very fast to maintain 120Hz.
