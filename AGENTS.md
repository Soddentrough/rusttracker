# RustTracker Agent Rules

## Targeted Verification for Visualizers

- When creating or modifying a visualizer (`src/shaders/*.wgsl`, `VisualizerDef` registration in `crates/rt-core/src/state.rs`, and visualizer pipeline wiring in `crates/rt-render/src/engine.rs`) without changing other core engine subsystems (`rt-audio`, `rt-decode`, `rt-tui`, or non-visualizer playback/DSP logic):
  - **Do NOT** run `cargo test --workspace`.
  - Run only targeted checks:
    1. `cargo run --bin validate_wgsl` (baseline WebGPU WGSL validation)
    2. Targeted unit tests for the visualizer registration/helpers (e.g., `cargo test -p rt-render <test_name>` and shader count tests if `src/shaders/` file count changed)
    3. Targeted GPU render/compute checks in `test_performance` (e.g., `cargo test --test test_performance <test_name>`)
- Only run the full `cargo test --workspace` suite prior to a release or when explicitly requested.
