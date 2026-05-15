# RustTracker TODO List

**Feature Request:**
Load `assets/neon_room.blend` to use as the basis for a new visualizer. It is a basic room scene with objects labelled after spatial channels (Front, LFE, center, rear, etc). 

The idea will be to load this scene and use a ray tracing engine to light up these objects in time to the audio streams.

**Implementation Notes:**
*   This may require the 'blend-rs' crate or some other system to load the 3d object data. It may require changes to the materials in Blender.

**Concerns:**
*   Binary file size bloat.
*   Computational intensity of a ray traced scene with dynamic lighting. Obviously only available on supported hardware. Must render very fast to maintain 120Hz.

## MacOS Bitstream/Passthrough Implementation

**Feature Request:**
Implement native bitstream passthrough for macOS to support spatial surround formats (AC3, E-AC3) directly via HDMI/Optical.

**Implementation Notes:**
*   macOS relies on CoreAudio. To support bitstreaming, we must write a HAL (Hardware Abstraction Layer) implementation.
*   Requires bypassing the macOS system mixer by seizing "Hog Mode" (`kAudioDevicePropertyHogMode`) on the audio device.
*   Must explicitly set the device's physical stream format to `kAudioFormat60958AC3` (or similar IEC 61937 sub-formats) to tell the OS to send raw frames.

**Concerns:**
*   Apple Silicon Macs are heavily restricted regarding HDMI audio passthrough. High-bandwidth lossless formats like TrueHD and DTS-HD Master Audio are likely to be stripped or decoded natively and may require EDID spoofers.
