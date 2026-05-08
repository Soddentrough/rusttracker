# Heatmap Debug Test Plan

We need to unequivocally prove whether the GPU is receiving the array data from the CPU, or if there is a logical flaw in how the WGSL shader calculates pixel coordinates. 

**Objective 1: Verify UV and Indexing Logic**
We will force the shader to draw a solid green gradient based purely on `y_idx`. If the gradient draws correctly from top to bottom, it proves the coordinate math (`uv.y` and `y_idx`) is perfectly healthy.

**Objective 2: Probe Absolute Buffer Addresses**
We will ignore dynamic indexing. We will ask the shader to read exactly one hardcoded index from the buffer (e.g., `heatmap_storage.history[0]`). We will instruct the shader: "If `history[0] > 10.0`, paint the entire screen Blue." If the CPU log says `row 0 > 10.0`, and the screen does not turn blue, we have 100% indisputable proof that WGPU/DX12 failed to transfer the memory. 
