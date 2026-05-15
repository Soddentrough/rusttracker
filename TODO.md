# RustTracker TODO List

**Feature Request:**
Load `assets/neon_room.blend` to use as the basis for a new visualizer. It is a basic room scene with objects labelled after spatial channels (Front, LFE, center, rear, etc). 

The idea will be to load this scene and use a ray tracing engine to light up these objects in time to the audio streams.

**Implementation Notes:**
*   This may require the 'blend-rs' crate or some other system to load the 3d object data. It may require changes to the materials in Blender.

**Concerns:**
*   Binary file size bloat.
*   Computational intensity of a ray traced scene with dynamic lighting. Obviously only available on supported hardware. Must render very fast to maintain 120Hz.
