# RustTracker Visualization System Review

Date: 2026-07-19. Scope: all 34 WGSL shaders in src/shaders/, the host-side
visualization pipeline in src/engine.rs (update/render), and src/state.rs
(visualizer registry). Review covers correctness, performance/overhead,
audio-reactivity, aesthetics, and UX.

=====================================================================
1. ARCHITECTURE OVERVIEW
=====================================================================

Pipeline structure:
- 22 visualizers registered in state.rs::VISUALIZERS, each declaring a
  pipeline type (FullscreenQuad or Mesh3D{grid|box, instances}) and
  requirement flags (history / fire / resynth / ferrofluidsim).
- ALL pipelines (22 render + ~8 compute) are created up front at startup
  (engine.rs:744-950). Mode switching is free; startup cost and VRAM are
  not.
- Per frame, engine.update() writes one AudioUniforms struct
  (spectrum[1024], fire_heat[1024], channels[32], phases, rects, timing)
  plus conditional uploads: waveform history (144x2048 f32 = 1.2MB),
  FireParams, gpu_spectrum (256KB), video planes.
- render() runs: heatmap compute (every frame), dummy FFT timestamp pass,
  resynth (if required), fire sim + buffer->texture copy (if required),
  neon smoke 64^3 (id 5), ferrofluidsim clear+physics (id 10), biolum
  (id 19), then one render pass: optional background -> visualizer ->
  video blit -> HUD -> egui.
- GPU timestamp queries with non-blocking readback (engine.rs:2608-2648)
  feed the stats overlay. Nicely done.
- Conditional work gating via requires_* flags is a good design; compute
  passes only run for the active visualizer.

The system is genuinely well-architected: shared _common.wgsl uniforms,
shared CRT/tonemap helpers, a clean registry, push-cadence-synchronized
scrolling (heatmap_row / step_fraction), and CPU-side physics where it
belongs (VU needle ballistics).

=====================================================================
2. CRITICAL / HIGH-SEVERITY FINDINGS
=====================================================================

[C1] Ferrofluidsim particle stride mismatch (DATA CORRUPTION)
  ferrofluidsim_compute.wgsl:6-11 declares Particle{pos:vec3, vel:vec3,
  mass:f32, smooth_spec:f32} = 48-byte WGSL stride, but the host allocates
  100_000 * 32 bytes (engine.rs:548). Result: indices >= ~66,666 are out
  of bounds; robust-access returns zeros, so ~1/3 of all particles take
  the init branch EVERY frame, freeze at deterministic positions, and
  splat a permanent static haze into the density grid. Only ~66.7k
  particles actually simulate; ~825k atomicAdds/frame are wasted.
  FIX: repack as two vec4s (pos.xyz+mass, vel.xyz+smooth_spec = exactly
  32B) or size the buffer 100_000*48.

[C2] Video: hardcoded viewport in VideoParams
  engine.rs:2562-2563 writes viewport_width/height = 1920.0/1080.0. The
  shader's aspect-fit math is correct but fed wrong dimensions on any
  non-1080p surface or resized window. Pass actual surface dims.

[C3] Video: write_texture row alignment
  engine.rs:2538-2555 passes the raw ffmpeg stride as bytes_per_row. wgpu
  requires bytes_per_row % 256 == 0; 8-bit 1920-wide luma (stride 1920)
  will fail validation/panic. 10-bit paths (stride*2) happen to pass.
  FIX: staging buffer or CPU repack when stride % 256 != 0.

[C4] All animation is framerate-dependent (systemic)
  - engine.rs:2238: play_time = push_count * 0.02 hardcodes a 50Hz push
    cadence, but audio.rs pushes history at target_fps (up to 144Hz).
    At 144fps smooth_time runs ~2.9x real-time: every time-based
    animation (synthwave ride, racer speed, rain fall, neon, fire)
    plays faster the higher the refresh rate.
  - Fixed dt=0.016 in biolum_compute (L135), ferrofluidsim (L47), and
    per-frame (non-dt-scaled) rates in fire_compute/firesim_compute mean
    flame height, ember lifetime, coal inertia, and fluid evolution all
    differ ~2.4x between 60Hz and 144Hz displays. The look is silently
    tuned to whatever refresh the author used.
  FIX: derive play_time from real elapsed time; pass true frame dt into
  the compute params and scale all rates.

[C5] Dead GPU FFT pipeline (~8.25MB VRAM + startup cost)
  gpu_fft.wgsl is still compiled and its pipeline/bind groups/8MB raw
  audio buffer/256KB spectrum buffer are still created (engine.rs:
  1789-1862), but engine.rs:4200-4209 replaced the dispatch with a dummy
  timestamp pass ("we now compute Cooley-Tukey FFT on CPU"). Note the
  shader is also not actually an FFT: it is a naive per-bin DFT with
  per-sample cos/sin and an "adaptive window" that makes bin magnitudes
  non-comparable across the spectrum — correctly abandoned.
  The _gpu_spectrum_buffer is live (CPU-filled, consumed by firesim /
  resynth / 3d-freq-scope); the shader and its buffers are dead.
  FIX: delete the shader + pipeline + raw-audio buffer, or feature-gate.
  Also rename state.gpu_fft — it now gates the CPU->GPU upload, not GPU
  FFT.

=====================================================================
3. CORRECTNESS BUGS (per module, ranked)
=====================================================================

- vis_synthwaveracer.wgsl:~650: lampposts every 76 units but road light
  pools computed at 24-unit spacing -> 2 of 3 light pools glow with no
  lamppost above them. Change 24.0 -> 76.0. Also: car march t_exit =
  t_entry+4.8 but bbox diagonal ~6.16 -> silhouette pinholes; use slab
  t_far.
- vis_oscilloscope.wgsl:136-137: phosphor_tint set to amber, then the
  already-amber-tonemapped color is multiplied by amber AGAIN in
  apply_crt_effects -> trace shifts to pure red, midtones crushed. Set
  tint to white for this shader.
- vis_3doscilloscope.wgsl:89/160: f32(num_lines - 1u) divides by zero
  when waveform_history_size == 1 -> NaN propagation. Guard with max(,1).
- vis_solar.wgsl:246-248: g_last_flare_dist read AFTER calcNormal, but
  calcNormal's 4 map() calls overwrite it -> flare distance from the
  last tetrahedron tap, not the hit point. Also L148-152: ejecta extend
  via fract(time*0.4+seed) sawtooth -> visible popping every ~2s.
- hud.wgsl:104: peak_segment = floor(peak*30); at full scale = 30 which
  only matches the exact top pixel row -> 0 dBFS peaks nearly invisible.
  Clamp to 29.
- vis_spatial.wgsl:257: label filler index 17u documented as "space" but
  glyph 17 is 'M' in _glyph_font.wgsl (latent; masked today because
  filler slots are never drawn). Use >= 19.
- vis_matrix.wgsl:174: u32(abs(col)) % num_channels — unguarded modulo
  by zero when no file is loaded.
- vis_spatial.wgsl:227 + smoke cs: sin-based hashes with unbounded time
  arguments lose f32 precision in long sessions (decorrelation/freeze).
- firesim_compute.wgsl:140: LFE spectrum offset missing the
  fft_ch % fft_channels guard applied to other channels -> reads garbage
  when fft_channels < num_channels.
- spectrum.wgsl:191-203 (UI scope mode): loop hardcodes 60 frames,
  ignores waveform_history_size; unwritten slots read as 0 -> bright
  amber line across mid-screen at startup. Phosphor decay calibrated for
  a stale 93ms push assumption (~3x too long at 144fps).
- vis_cuboids.wgsl:115-124: the "wireframe edge detection" is a
  mathematical no-op (fragment lies ON a face, so min-distance is ~0)
  -> whole-face glow, not wireframe. Either fix (exclude face-normal
  axis, measure 2D edge distance) or simplify and embrace the look.
- resynth_compute.wgsl:33: eq_boost sqrt(freq/20) reaches ~33x at top
  bins with only /90.5 normalization -> HF material can exceed +/-1 and
  clip downstream. Also magnitude-only "phase lock" comment is
  misleading (synthetic fixed phase, not signal phase).
- Reversed-edge smoothstep (edge0 > edge1) is used in ~20 locations
  (_common:101, ferrofluid:148/227, synthwave, racer, rain, 3drain...).
  WGSL says the result is indeterminate; naga currently lowers it to the
  clamped form so it works everywhere today, but it is spec-UB. Add a
  smoothstep_r(hi, lo, x) helper to _common.wgsl and migrate.
- heatmap_compute.wgsl:30-33: pre-fills the "future" ring row each frame
  -> oldest visible row silently duplicates the newest. Acceptable, but
  document. The steps>16 clamp can leave a stale tear band after hitches.

=====================================================================
4. PERFORMANCE / OVERHEAD
=====================================================================

Host-side per-frame waste:
- engine.rs:2402: the full 1.2MB waveform history buffer is re-uploaded
  EVERY frame when the visualizer requires history — even when paused or
  when no new rows were pushed. Skip when push_count unchanged.
- engine.rs:2448: 256KB gpu_spectrum uploaded every frame whenever
  gpu_fft flag is set, regardless of whether any consumer is active.
  Gate on the active visualizer actually reading it.
- Two write_buffer calls per frame to the same uniform buffer (full
  struct in update(), aspect patch in render(), engine.rs:2374/4371).
  Trivial, but the aspect could be written once.
- CPU pre-smoothing of 144x2048 waveform rows (engine.rs:2391-2399)
  runs every frame even when unchanged. Same fix as above.

Shader cost tiers (approximate, fullscreen at high res):
  EXTREME: vis_3doscilloscope (~1870 segment evals/px worst case) —
    superseded by vis_3doscilloscope_raster which achieves the same look
    at a tiny fraction of the cost (36k verts + cheap fragment).
    Recommend deprecating the raymarched version or capping at 72 lines.
  HEAVY: vis_neon (100 march steps, per-pixel scene setup + smoke
    texture fetches + 8-step reflection march), vis_ferrofluidsim
    (~1450 scattered storage loads/px worst case), vis_solar,
    vis_ferrofluid, vis_vumeters (80-step march, background pixels burn
    all 80 steps for black), vis_oscilloscope (~936 sdLine/px worst).
  MODERATE: vis_firesim renderer (unconditional 5-octave sin-hash fbm
    per pixel even for black background — add early-out), vis_starfield
    (25 layers + 4-octave nebula fbm), vis_3drain (spiky lightning),
    vis_synthwaveracer (~100 box tests/px, no early-out),
    vis_lissajous (240k verts/frame — 200x200 cross-section for a hair-
    thin tube is ~15x overkill; use 200x12).
  LIGHT: everything else (spectrum, matrix, spatial, rain, cuboids,
    flame renderer, biolum render*, synthwave*, 3d-osc-raster).
    *biolum wastes 4x vertex work walking a 36-vert box and discarding
    30 verts for a billboard; synthwave's streetlamp loop (10 segments
    x 2 lamps with terrain noise) runs for every pixel incl. sky.

Easy wins:
- vis_vumeters: ray-AABB reject before marching (~99% cut on background).
- vis_ferrofluid: sky early-out (rd.y>0 && p.y>1.1); replace exp()
  falloff with rational falloff; STEP_SCALE 0.45 is marginally
  Lipschitz-unsafe at vu=1 -> spike flicker on loud transients.
- vis_neon: move setup_scene/act to a per-frame uniform; steps 100->64;
  skip smoke when activity ~0. Also jitter seed uses fract(smooth_time)
  -> dither pattern pops every 1s.
- biolum: use a 4-vert quad; reuse Gerstner displacement instead of 3x
  eval; add tonemap (currently none -> harsh clipping at peaks).
- firesim: replace sin-hash perlin with the pcg hash already in the file
  (~2-3x on the sim); render early-out on black.
- solar: move setup_flares() to a CPU uniform (removes ~150 trig/pixel).
- synthwave: halve lamp segments, add V.z early-out, reduce 16 heatmap
  taps/vertex to 8, add ACES tonemap (fs_main has none -> lamp/shoulder
  HDR clips).
- lissajous: cut tube cross-section tessellation ~15x; hardcoded camera
  eye in fragment shader (L232) will silently desync from the engine
  camera; dead CameraUniforms binding in 3d-osc-raster (L5) likewise.

Startup/memory:
- All 22 shaders compiled at launch. Consider lazy compilation on first
  selection (background thread) to cut startup time.
- Delete dead gpu_fft buffers (8.25MB) and vis_3dtest.wgsl (not
  registered, and its bind layout is incompatible if it ever were).

Steam Deck validation:
- The disabled set (2, 7, 8, 4, 10, 5) is mostly justified. Notes:
  - vis_3doscilloscope_raster (id 20) is Deck-viable and is the right
    replacement for ids 7/8 — consider enabling it and keeping 7/8 off.
  - vis_ferrofluid (4) could become viable with the sky early-out +
    cheaper falloff; ferrofluidsim (10) needs 0.5x internal res.
  - A general "render at 0.5-0.75x and upscale" quality tier would let
    the soft/bloomy shaders (solar, neon, ferro*) run on Deck.

=====================================================================
5. AESTHETICS & UX
=====================================================================

Strengths:
- Strong family identity: amber phosphor CRT language across the
  oscilloscope family; retro authenticity in vis_flame (hard 8-step
  palette, chunky pixels, phosphor triads — period-accurate);
  showcase-quality vis_vumeters (skeuomorphic Denon-style meters with
  physically-modelled needle ballistics, vector-stroke dial text);
  vis_3doscilloscope_raster is the most polished technical piece (DOF,
  chromatic wire, aperture grille, glass specular, rock-stable scroll).
- Genuinely musical mappings: per-channel lightning bolts (rain),
  frequency-to-depth mapping (3drain), speaker-azimuth ferrofluid
  spikes, per-band resynthesized waveforms (3d freq scope — arguably the
  most "musical" visualizer), world-locked scrolling terrain
  (synthwave's 0.5 units/row matches the heatmap ring exactly).
- UX details done right: inactive speakers rendered as grey labelled
  orbs (spatial), idle needle sway when no file is loaded, foam
  self-excitation keeps biolum alive in quiet passages, non-blocking
  GPU timing readback, per-visualizer enable list + rotation picker.

Weaknesses / recommendations:
- Framerate dependence (C4) is also an aesthetic bug: the same track
  looks materially different at 60Hz vs 144Hz. Highest-priority fix for
  visual consistency.
- Unbounded f32 time: cam_z reaches ~90k after 1h (synthwave L75) where
  f32 epsilon ~0.008 -> visible jitter. Wrap time modulo a large period.
- Palette cohesion: lissajous's rainbow hue-cycling center scope is the
  most fatiguing element in the app (suggest sat ~0.7); 3drain's white
  bolts are less atmospheric than rain's blue-white; ferrofluid's
  per-channel candy glow can turn rave on loud multichannel material.
- Missing global reactivity: starfield computes bass/mid then never
  uses them (no warp-on-bass); hud fire widget has zero audio
  reactivity; rain density/speed is static; lissajous shapes dance at
  fixed speed during silence (damp phase speed by band energy).
- spectrum.wgsl scope-mode startup artifact (bright amber line) and
  vis_spectrum's flat per-column color (color by LED height for the
  classic gradient look; add peak-hold caps).
- Silence behavior: several shaders freeze or dance at fixed amplitude
  with no input; a consistent "calm idle" language would help.
- Small-text legibility: 3x5 glyph labels (spatial) and 0.026-scale VU
  percent labels shimmer at small sizes — add fwidth-based AA in
  draw_label_char.
- vis_firesim's "Debug Labels" block is always on — it's actually a
  good UX feature (channel identification); rename and add a toggle.
- Aspect ratio is derived three different ways (dpdx/dpdy trick,
  hardcoded 1.77/1.7777 fallbacks, audio.aspect_ratio uniform). The
  hardcoded fallbacks are wrong on 16:10/ultrawide. Standardize on the
  uniform.
- VU meters only ever show 2 channels even for 8ch files (hardcoded
  actual_num=2) — fine as a design choice, but document it; a
  multi-meter layout would be a nice upgrade.
- heatmap/firesim thresholds and magic scales (0.015, /100, /30) are
  coupled to CPU FFT normalization with no shared constants — fragile.

=====================================================================
6. PRIORITIZED RECOMMENDATIONS
=====================================================================

P0 (correctness/data):
1. Fix ferrofluidsim Particle stride (48B vs 32B) — phantom particles.
2. Video: real viewport dims; 256-byte row-alignment guard.
3. Framerate-independent timing: real elapsed play_time + true dt in all
   compute sims (fire, firesim, biolum, ferrofluidsim).
4. Fix racer lamp-pool spacing (24->76) and car t_exit.

P1 (robustness/quick fixes):
5. Delete dead GPU FFT pipeline + buffers; delete vis_3dtest.wgsl.
6. Add smoothstep_r helper; migrate reversed-edge call sites.
7. Oscilloscope double-amber tint; 3d-osc div-by-zero; hud peak clamp;
   solar g_last_flare_dist ordering + ejecta sawtooth; spectrum.wgsl
   startup line + history-size loop; spatial 'M'-as-space; matrix mod-0;
   firesim LFE guard; resynth eq soft-knee.
8. Skip waveform/gpu_spectrum uploads when unchanged.

P2 (performance):
9. Deprecate raymarched vis_3doscilloscope in favor of the raster
   variant (or cap at 72 lines); enable raster on Deck.
10. vumeters ray-AABB reject; ferrofluid sky early-out; neon uniform
    scene setup + steps 100->64; firesim black early-out + pcg noise;
    biolum 4-vert quads; lissajous 15x tessellation cut.
11. Half-res render tier for heavy shaders (Deck viability).
12. Standardize on audio.aspect_ratio uniform everywhere.

P3 (aesthetics/UX):
13. Wrap smooth_time to fix long-session precision decay.
14. Wire up starfield bass->warp; audio-reactive rain density; damp
    lissajous during silence; onset-triggered (not level-gated) 3drain
    lightning with cooldown (also a mild photosensitivity fix).
15. Tone down lissajous rainbow saturation; tint 3drain bolts;
    desaturate ferrofluid channel glow; add tonemap to biolum/synthwave.
16. LED-height coloring + peak caps for spectrum; glyph AA; firesim
    label toggle; document VU stereo-pair design.
