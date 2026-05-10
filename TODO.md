# RustTracker TODO List

## Version 0.8.8: Scalable panels

** Feature Request:**

Without adding any visible borders, the user should be able to dynamically resize the visualizer by dragging the horizontally intersecting line up/down. This would rescale the top three panels (or video window if active) and dynamically scaling the vizualization's graphics.
If the user does not have a mouse then this should be acheived by using a gamepad's right stick. Moving the stick up and down would rescale the top three panels (or video window if active) and dynamically scaling the vizualization's graphics in the bottom panel.


## Version 0.9: Neon Room Ray Traced Visualizer

**Feature Request:**
Load `assets/neon_room.blend` to use as the basis for a new visualizer. It is a basic room scene with objects labelled after spatial channels (Front, LFE, center, rear, etc). 

The idea will be to load this scene and use a ray tracing engine to light up these objects in time to the audio streams.

**Implementation Notes:**
*   This may require the 'blend-rs' crate or some other system to load the 3d object data. It may require changes to the materials in Blender.

**Concerns:**
*   Binary file size bloat.
*   Computational intensity of a ray traced scene with dynamic lighting. Obviously only available on supported hardware. Must render very fast to maintain 120Hz.
