# RustTracker Project Audit

Date: 2026-10-04 (v0.9.27, HEAD `2677aac`)

Scope: README/TODO/VIS_REVIEW, `Cargo.toml`, CI workflows, git history, and spot checks of `src/` (audio, engine, main, shaders).

> [!NOTE]
> This is a static review. The build, tests, clippy and profiling were **not** run. Performance items are hypotheses to verify with profiling. Line numbers are approximate.

## 1. Summary

RustTracker is a Vulkan/wgpu multichannel audio visualizer and tracker player. It targets Linux, Windows, macOS, Android and Steam Deck. It is about 24k lines of Rust plus 45 WGSL shaders, with a WASAPI/bitstream passthrough path.

**Strengths**
- Current stack: wgpu 29, egui 0.34, edition 2024, cpal 0.17, symphonia 0.5.
- Audio callback uses `try_lock` / `try_recv` instead of blocking.
- Non-blocking GPU timestamp readback, and `requires_*` flags that gate compute work to the active visualizer.
- Strong visual identity across the visualizers.

**Weaknesses**
- Monolithic files: `engine.rs` 8.1k lines, `audio.rs` 4.6k, `main.rs` 2.3k.
- CI only builds and packages. It runs no tests, clippy, or fmt.
- Many `unwrap`/`expect` calls, and suppressed warnings including deprecations.
- The repo is cluttered with build debris and copied third-party code.

**Previous review:** `VIS_REVIEW.md` (2026-07-19) listed P0 bugs.
- Verified fixed: ferrofluid particle stride (shader and host are both 32 B), and video row-alignment repack (`engine.rs` ~4332).
- Not verified: the rest. The document has no status tracking, so archive it or add a status column.

## 2. Critical bugs / risks

1. **Crash when X11 is unavailable.** *(RESOLVED)*: Replaced forced `WINIT_UNIX_BACKEND=x11` with native Wayland first + fallback to X11, and replaced `create_window().unwrap()` with graceful error reporting.
2. **Widespread `unwrap`/`expect`.** Counts of `unwrap`/`expect`/`unsafe`-related matches: `main.rs` 93, `bitstream.rs` 55, `engine.rs` 47, `android.rs` 37. Window creation (`main.rs:349`) is now handled gracefully; remaining GPU-init and audio call sites should continue to be hardened.
3. **Hardcoded 16:9 aspect fallbacks** (`engine.rs:2555` `1.7777`, plus shaders). Wrong on 16:10 and ultrawide. Use the `audio.aspect_ratio` uniform everywhere.
4. **Framerate-dependent simulation** (VIS_REVIEW C4). RustTracker now defaults to display refresh rate pacing with `--uncapped` opt-in flag; compute passes should be migrated to real `dt` scaling.
5. **`wgpu::Limits::default()`** (`engine.rs:1666`). Request the limits and features actually needed, and fail gracefully.
6. **Unbounded f32 time** (e.g. synthwave `cam_z`, ~90k after 1 h) causes jitter. Wrap time.
7. **Uncommitted working-tree state.** `dist/.crates.toml`, `dist/.crates2.json` and `dist/bin/rusttracker` are staged for deletion, and `.gitignore` is modified. Review before committing. `*.txt` is globally ignored, which would hide any real `.txt` file.

## 3. Minor bugs / hygiene

- **Warning suppression.** 30+ `#[allow(...)]`: `dead_code` (many in `engine.rs`), `unused_variables`/`unused_assignments` on large functions (`main.rs:253`, `:1970`), `unused_imports`.
- **Deprecated winit API hidden.** `#[allow(deprecated)]` at `main.rs:348` and `:409` (`create_window`, `EventLoop::run`). Migrate to `ApplicationHandler`/`ActiveEventLoop`.
- **Dead code.** Unused GPU FFT pipeline and buffers (~8 MB) and `vis_3dtest.wgsl` (per VIS_REVIEW). Delete them.
- **Repo debris (tracked).** `ad_spdif.c`, `ao_wasapi*.c` (third-party source: check licence and provenance), `fix_struct.py`, `rewrite_bitstream.py`, `scratch_bitstream*` crates, `icon_original.png`, `src/test.rs`, `src/test_egui.rs`, `src/test_wgpu.rs`.
- **Repo debris (untracked).** `*.AppImage` (about 150 MB), `extracted_deb/`, `release-artifacts/`, `windows_release_0.9.22/`, `plan.txt`, `scratch.*`, logs. `.git` is 255 MB.
- **Test and probe binaries are built as products:** `src/bin/test_ffmpeg_seek.rs`, `validate_wgsl.rs`, `verify_3d_font.rs`. Move to `examples/` or an xtask.
- **Magic numbers.** FFT-normalization scales in shaders (0.015, /100, /30) coupled to CPU code, `100_000 * 32` particles, `1024 * 4`. Centralize and generate the WGSL header from shared constants.
- **Doc inconsistencies.**
  - `Cargo.toml` description says "Tracker Module Visualizer", but the app also handles video, mic, MIDI and bitstream.
  - `TODO.md` contradicts itself on `FullscreenQuad`.
  - Visualizer counts disagree: VIS_REVIEW says 22 visualizers, and the shader directory has 45 files.
- **Duplicated logic.** `is_game_mode` detection (`main.rs:319`, `engine.rs:5127`) and `HOME`/`USERPROFILE` lookups (`ui.rs`, `main.rs`, `engine.rs`). Use the `dirs` crate.
- **Env var sprawl.** `RUSTTRACKER_{FPS_LIMIT,PROFILE,PIPELINE_CACHE,CACHE_DIR,PRESENT_MODE,WAYLAND}` are read ad hoc. Move to clap flags or a config file, and document them.
- **Release profile.** `strip = false`, `debug = 1`, thin LTO. Use a separate dist profile.
- **Pipeline cache.** Confirm the key includes adapter, driver version and shader hash.

## 4. Optimizations

**Build**
- Use `lto = "fat"`, `codegen-units = 1`, and `panic = "abort"` where compatible.
- `x86-64-v3` is set for Linux and Windows only. Consider `target-cpu=native` for local builds (the Threadripper 3970X has no AVX-512).
- Use a faster linker (mold) for dev builds.

**Audio / DSP**
- Evaluate `realfft` (real-to-complex) instead of complex `rustfft` for real input.
- 47 mutex sites in `audio.rs`. Move visualizer data to a triple buffer / `arc-swap` / atomics. When paused, the callback does `try_lock` + `fill(0.0)` on every call.
- Confirm no float round trip or SRC in passthrough / bitstream paths.

**GPU**
- Skip per-frame uploads when unchanged: waveform history (144×2048 f32, 1.2 MB) and gpu_spectrum (256 KB). Consider f16 history (`SHADER_F16`).
- Move large uniforms (spectrum[1024], fire_heat[1024]) to a storage buffer ring.
- Use subgroup operations and request features where available.
- Lazy-compile shaders (45 at launch), backed by the pipeline cache.
- Add a render-scale tier (0.5–0.75×) to make heavy shaders viable on Steam Deck and mobile.
- Per-pixel shader early-outs (from VIS_REVIEW): vumeters ray-AABB reject, ferrofluid sky early-out, neon and solar scene setup moved to per-frame uniforms, firesim black early-out, biolum 4-vertex quads, lissajous tessellation cut.
- Default to vsync or a refresh-rate/VRR cap. Keep uncapped as a benchmark mode.
- Video: use hardware decode and zero-copy import (VAAPI / Vulkan Video, dmabuf) instead of CPU YUV upload via `write_texture`.

## 5. Testing and CI

- CI runs `cargo build --release` plus packaging only. Add `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and `cargo audit` or `cargo deny`.
- `tests/` (~1.7k lines) is mostly benchmark and probe style. Convert to `#[test]` or Criterion benches.
- `test_baselines/` is gitignored, so golden-image tests can't run in CI.
- Add:
  - headless wgpu tests (lavapipe) for pipeline creation;
  - WGSL validation via the existing `validate_wgsl`;
  - fuzzing for tracker, MIDI and lyrics parsers;
  - property tests for ring buffers and resampling.
- Pin FFmpeg versions in CI. The history shows fragile MSYS2 header patching.

## 6. Architecture recommendations

1. Split `engine.rs` into context, pipelines (registry-driven), uploads, visualizers and HUD. Replace hard-coded visualizer IDs (5, 10, 19) with a trait or registry.
2. Split `audio.rs` into decoder, output backends, DSP/FFT and tracker engine.
3. Put platform audio backends (WASAPI, CoreAudio, Android, cpal) behind a trait to reduce `cfg` sprawl.
4. Adopt winit `ApplicationHandler`.
5. Use typed errors (`thiserror`) at module boundaries.
6. Add a TOML config file (visualizer enable list, devices, presets).
7. Review dependencies: `egui-file-dialog` against `rfd`, and the maintenance status of `rustysynth` and `midly`.
8. Decide the 3D direction (TODO.md: native `Mesh3D`) per visualizer, based on profiling. Raymarching remains appropriate for ferrofluid and metaball looks.

## 7. Nice-to-have features

- HDR output (PQ / Rec.2020) for neon and fire shaders.
- Loudness and spatial metering: EBU R128 LUFS and true peak, per-channel correlation, phase scope, object-audio metadata display.
- Onset/beat detection (spectral flux) to drive visuals, replacing level-gated lightning. Add a photosensitivity-safe mode (flash-rate limit).
- PipeWire/JACK native output and WASAPI event mode for low latency.
- User shader presets with WGSL hot reload (naga is already a dependency).
- MIDI/OSC control, plus NDI / PipeWire video output for streaming.
- Mel/CQT spectrograms, perceptual weighting, and GPU stem separation (Demucs/ONNX) for stem-driven visuals.
- Accessibility: reduced-motion, high-contrast HUD, scalable UI.
- Release hygiene: signed releases, Flatpak (a metainfo file exists), changelog.

## 8. Suggested priority order

| Priority | Items | Status |
| :--- | :--- | :--- |
| Quick wins | Wayland-then-X11 fallback & graceful window errors. Default to VSync/refresh cap with `--uncapped` flag. Fix clippy warnings & format codebase. Add fmt, clippy, WGSL validate, and test to CI. | **Done (Milestone 1)** |
| Repo hygiene | Repo cleanup (debris out, `.git` pruning or LFS). Remove `allow(dead_code)` and delete dead shaders/buffers. | In Progress |
| Medium | Framerate-independent simulation (`dt`). Explicit wgpu limits & features (`SHADER_F16`, `SUBGROUP`). Wrap time values. Replace hardcoded aspect ratios. Centralize constants & config. | Planned |
| Larger | Split `engine.rs` and `audio.rs`. Move to `ApplicationHandler`. Headless golden-image tests and parser fuzzing. Hardware video decode and render-scale tier. Mesh3D migration per visualizer. | Planned |
