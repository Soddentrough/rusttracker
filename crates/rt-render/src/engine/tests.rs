use super::meshes::EMBEDDED_GLASS_FONT;
use super::*;

fn engine_prod_source() -> String {
    let engine_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine");
    let mut entries: Vec<_> = std::fs::read_dir(&engine_dir)
        .expect("crates/rt-render/src/engine must exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| e == "rs")
                && p.file_name().is_none_or(|n| n != "tests.rs")
        })
        .collect();
    entries.sort();
    let mut out = String::new();
    for path in entries {
        out.push_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display())),
        );
        out.push('\n');
    }
    out
}

fn workspace_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

impl SplashCache {
    fn build_dynamic_bg_mesh(&self, rect: egui::Rect, time: f32, ppp: f32) -> egui::Mesh {
        let mut mesh = egui::Mesh::default();
        self.append_dynamic_bg(&mut mesh, rect, time, ppp);
        mesh
    }
}

#[test]
fn test_uniform_size_alignment() {
    let rust_size = std::mem::size_of::<AudioUniforms>();
    assert_eq!(
        rust_size, 9856,
        "Rust AudioUniforms size is not 9856 bytes (actual: {})",
        rust_size
    );

    // Parse _common.wgsl to compute WGSL structure size
    let common_source = std::fs::read_to_string(workspace_path("src/shaders/_common.wgsl"))
        .expect("Failed to read _common.wgsl");

    // Parse AudioUniforms struct fields
    let struct_content = common_source
        .split("struct AudioUniforms {")
        .nth(1)
        .expect("Could not find struct AudioUniforms in _common.wgsl")
        .split("};")
        .next()
        .expect("Could not find end of struct AudioUniforms");

    let mut offset: usize = 0;
    for line in struct_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 2 {
            continue;
        }
        let ty = parts[1].trim().trim_end_matches(',');

        // Determine size and alignment
        let (size, align) = if ty.starts_with("array<") {
            if ty.contains("vec4<") {
                let count_str = ty.split(',').nth(1).unwrap().trim().trim_end_matches('>');
                let count: usize = count_str.parse().unwrap();
                (count * 16, 16)
            } else {
                panic!("Unknown array type in shader: {}", ty);
            }
        } else {
            match ty {
                "u32" | "i32" | "f32" => (4, 4),
                "vec2<f32>" | "vec2<u32>" | "vec2<i32>" => (8, 8),
                "vec3<f32>" | "vec3<u32>" | "vec3<i32>" => (12, 16),
                "vec4<f32>" | "vec4<u32>" | "vec4<i32>" => (16, 16),
                _ => panic!("Unknown type in shader: {}", ty),
            }
        };

        // Align the offset
        offset = offset.div_ceil(align) * align;
        offset += size;
    }

    // Align struct size to maximum alignment (16)
    let wgsl_size = offset.div_ceil(16) * 16;
    assert_eq!(
        wgsl_size, 9856,
        "WGSL AudioUniforms size is not 9856 bytes (actual: {})",
        wgsl_size
    );
    assert_eq!(
        rust_size, wgsl_size,
        "Size mismatch: Rust AudioUniforms is {}, WGSL is {}",
        rust_size, wgsl_size
    );
}

#[test]
fn test_timing_and_scroll_stability() {
    let mut smooth_dt = 1.0f64 / 60.0f64;
    let mut smooth_time = 0.0f64;
    let mut raw_time = 0.0f64;

    let mut raw_deltas = Vec::new();
    let mut smooth_deltas = Vec::new();

    // Simulate 100 frames with CPU scheduling noise
    for i in 0..100 {
        // Deterministic pseudo-random jitter between 13ms and 20ms using a sine phase
        let phase = (i as f64) * 0.73;
        let jitter = phase.sin() * 0.0035; // +/- 3.5ms
        let dt = 0.01667 + jitter;

        let prev_raw = raw_time;
        raw_time += dt;
        raw_deltas.push(raw_time - prev_raw);

        let alpha = 0.03f64;
        let prev_smooth = smooth_time;
        smooth_dt = smooth_dt * (1.0 - alpha) + dt.clamp(0.001, 0.1) * alpha;
        smooth_time += smooth_dt;
        smooth_deltas.push(smooth_time - prev_smooth);
    }

    // Calculate second derivative (acceleration / velocity change) of the timeline
    let mut raw_accelerations = Vec::new();
    let mut smooth_accelerations = Vec::new();
    for i in 1..99 {
        let acc_raw = raw_deltas[i] - raw_deltas[i - 1];
        let acc_smooth = smooth_deltas[i] - smooth_deltas[i - 1];
        raw_accelerations.push(acc_raw.abs());
        smooth_accelerations.push(acc_smooth.abs());
    }

    let max_raw_acc = raw_accelerations.iter().cloned().fold(0.0, f64::max);
    let max_smooth_acc = smooth_accelerations.iter().cloned().fold(0.0, f64::max);

    println!("Max raw frame-to-frame velocity jump: {:.6}s", max_raw_acc);
    println!(
        "Max smoothed frame-to-frame velocity jump: {:.6}s",
        max_smooth_acc
    );

    // Smooth timelines must have at least 15x lower frame-to-frame velocity jumps
    assert!(
        max_smooth_acc < max_raw_acc / 15.0,
        "The timing filter did not damp frame-to-frame velocity changes sufficiently: smooth={:.6}, raw={:.6}",
        max_smooth_acc,
        max_raw_acc
    );
}

#[test]
fn test_drag_and_drop_use_cases() {
    let mut state = crate::state::AppState::new(String::new());
    let track_info_rect = [800.0f32, 0.0, 1200.0, 300.0];
    state.track_info_rect = Some(track_info_rect);

    let cursor_inside = Some([950.0f32, 100.0]);
    let cursor_outside = Some([400.0f32, 150.0]);

    // Use Case 1: Nothing is playing (file_loaded == false), even if dropped on Track Info -> play immediately
    assert!(!state.file_loaded);
    state.handle_dropped_files(vec!["song1.flac".to_string()], cursor_inside);
    assert!(state.file_loaded);
    assert!(!state.is_paused);
    assert!(!state.track_ended);
    assert_eq!(*state.playlist, vec!["song1.flac".to_string()]);
    assert_eq!(state.playlist_index, 0);
    assert_eq!(state.load_request.take(), Some("song1.flac".to_string()));
    state.song_title = "song1.flac".to_string();

    // Use Case 2: Single track is playing, drop onto Track Info -> creates playlist with dropped track as next track
    state.handle_dropped_files(vec!["song2.mp3".to_string()], cursor_inside);
    assert_eq!(
        *state.playlist,
        vec!["song1.flac".to_string(), "song2.mp3".to_string()]
    );
    assert_eq!(state.playlist_index, 0);
    assert_eq!(state.playlist[state.playlist_index + 1], "song2.mp3");
    assert_eq!(state.load_request, None);
    assert_eq!(
        state.osd_text,
        Some("Added to Playlist: song2.mp3".to_string())
    );

    // Use Case 3: Playlist is playing track 0 ("song1.flac"), drop "song_next.wav" onto Track Info
    // -> inserts "song_next.wav" as the next track (index 1) before "song2.mp3"
    state.handle_dropped_files(vec!["song_next.wav".to_string()], cursor_inside);
    assert_eq!(
        *state.playlist,
        vec![
            "song1.flac".to_string(),
            "song_next.wav".to_string(),
            "song2.mp3".to_string()
        ]
    );
    assert_eq!(state.playlist_index, 0);
    assert_eq!(state.playlist[state.playlist_index + 1], "song_next.wav");
    assert_eq!(state.load_request, None);
    assert_eq!(
        state.osd_text,
        Some("Added to Playlist: song_next.wav".to_string())
    );

    // Use Case 4: Drop outside Track Info while playing -> replaces playlist and plays immediately
    state.handle_dropped_files(vec!["song3.ogg".to_string()], cursor_outside);
    assert_eq!(*state.playlist, vec!["song3.ogg".to_string()]);
    assert_eq!(state.playlist_index, 0);
    assert_eq!(state.load_request.take(), Some("song3.ogg".to_string()));
    state.song_title = "song3.ogg".to_string();

    // Use Case 5: Playback has ended (track_ended == true), drop onto Track Info -> plays immediately
    state.track_ended = true;
    state.is_paused = true;
    state.handle_dropped_files(vec!["song4.flac".to_string()], cursor_inside);
    assert!(!state.track_ended);
    assert!(!state.is_paused);
    assert_eq!(*state.playlist, vec!["song4.flac".to_string()]);
    assert_eq!(state.playlist_index, 0);
    assert_eq!(state.load_request.take(), Some("song4.flac".to_string()));
}

#[test]
fn test_view_mode_rotation() {
    let mut state = crate::state::AppState::new("Test App".to_string());
    assert!(state.show_hud);
    assert_eq!(state.video_mode, 0);

    // Press 'v' in audio mode -> Toggle HUD off (Full Screen Visualizer)
    state.show_hud = !state.show_hud;
    state.video_mode = 0;
    assert!(!state.show_hud);

    // Press 'v' again -> Toggle HUD back on (Standard View)
    state.show_hud = !state.show_hud;
    state.video_mode = 0;
    assert!(state.show_hud);
}

#[test]
fn test_synthwave_lyrics_visualizer_registration() {
    let vis = crate::state::VISUALIZERS.iter().find(|v| v.id == 23);
    assert!(
        vis.is_some(),
        "Visualizer ID 23 (3D Glass Water Lyrics) must be registered in VISUALIZERS"
    );
    let def = vis.unwrap();
    assert_eq!(def.name, "3D Glass Water Lyrics");
    assert_eq!(def.filename, "vis_lyrics.wgsl");
    assert_eq!(
        def.pipeline_type,
        crate::state::PipelineType::Mesh3D {
            geometry: crate::state::Geometry::GlassLyricsScene,
            instances: 1
        }
    );

    // Verify that the shader source is present and includes compile cleanly
    let raw_source = include_str!("../../../../src/shaders/vis_lyrics.wgsl");
    assert!(raw_source.contains("// INCLUDE: common"));
    assert!(raw_source.contains("vs_main_3d"));
    assert!(raw_source.contains("fs_main"));
    assert!(raw_source.contains("camera.view_matrix"));

    // Verify ID 23 is disabled by default in AppState
    let state = crate::state::AppState::new("Test App".to_string());
    let lyrics_idx = crate::state::VISUALIZERS
        .iter()
        .position(|v| v.id == 23)
        .unwrap();
    assert!(
        !state.vis_enabled[lyrics_idx],
        "3D Glass Water Lyrics (ID 23) should be disabled by default"
    );
}

#[test]
fn test_mobile_visualizer_fixes() {
    // Verify Midnight Storm shader (vis_rain.wgsl) has the updated reactive lightning logic
    let rain_source = include_str!("../../../../src/shaders/vis_rain.wgsl");
    assert!(rain_source.contains("lightning_bolt"));
    assert!(rain_source.contains("strike_tick"));
    assert!(rain_source.contains("audio.smooth_time"));
    assert!(rain_source.contains("channel_peaks"));

    // Verify VU meter rack OBJ model loads cleanly and has non-empty geometry
    let (vu_verts, vu_indices) = generate_vumeter_rack_mesh();
    assert!(
        !vu_verts.is_empty(),
        "VU meter rack vertices must not be empty"
    );
    assert!(
        !vu_indices.is_empty(),
        "VU meter rack indices must not be empty"
    );

    // Verify Glass Lyrics mesh generator handles long strings with appropriate scaling
    let (lyrics_verts, lyrics_indices) =
        generate_glass_lyrics_mesh("THIS IS A VERY LONG TEST LYRIC LINE FOR MOBILE VIEWPORT");
    assert!(
        !lyrics_verts.is_empty(),
        "Glass lyrics vertices must not be empty for long line"
    );
    assert!(
        !lyrics_indices.is_empty(),
        "Glass lyrics indices must not be empty for long line"
    );

    const SHADER_COMMON: &str = include_str!("../../../../src/shaders/_common.wgsl");
    const SHADER_GLYPH_FONT: &str = include_str!("../../../../src/shaders/_glyph_font.wgsl");
    let resolve_shader_includes = |source: &str| -> String {
        source
            .replace("// INCLUDE: common", SHADER_COMMON)
            .replace("// INCLUDE: glyph_font", SHADER_GLYPH_FONT)
    };
    let rain_resolved =
        resolve_shader_includes(include_str!("../../../../src/shaders/vis_rain.wgsl"));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&rain_resolved) {
        panic!("vis_rain WGSL error:\n{}", e.emit_to_string(&rain_resolved));
    }

    let vu_resolved =
        resolve_shader_includes(include_str!("../../../../src/shaders/vis_vumeters_3d.wgsl"));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&vu_resolved) {
        panic!(
            "vis_vumeters_3d WGSL error:\n{}",
            e.emit_to_string(&vu_resolved)
        );
    }

    let lyrics_resolved =
        resolve_shader_includes(include_str!("../../../../src/shaders/vis_lyrics.wgsl"));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&lyrics_resolved) {
        panic!(
            "vis_lyrics WGSL error:\n{}",
            e.emit_to_string(&lyrics_resolved)
        );
    }

    let crt_scope_resolved = resolve_shader_includes(include_str!(
        "../../../../src/shaders/vis_oscilloscope.wgsl"
    ));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&crt_scope_resolved) {
        panic!(
            "vis_oscilloscope WGSL error:\n{}",
            e.emit_to_string(&crt_scope_resolved)
        );
    }

    let scope_resolved = resolve_shader_includes(include_str!(
        "../../../../src/shaders/vis_3doscilloscope_raster.wgsl"
    ));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&scope_resolved) {
        panic!(
            "vis_3doscilloscope_raster WGSL error:\n{}",
            e.emit_to_string(&scope_resolved)
        );
    }

    let resynth_resolved =
        resolve_shader_includes(include_str!("../../../../src/shaders/resynth_compute.wgsl"));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&resynth_resolved) {
        panic!(
            "resynth_compute WGSL error:\n{}",
            e.emit_to_string(&resynth_resolved)
        );
    }

    let freq_scope_resolved = resolve_shader_includes(include_str!(
        "../../../../src/shaders/vis_3doscilloscope_freq.wgsl"
    ));
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&freq_scope_resolved) {
        panic!(
            "vis_3doscilloscope_freq WGSL error:\n{}",
            e.emit_to_string(&freq_scope_resolved)
        );
    }
    let freq_vis = crate::state::VISUALIZERS
        .iter()
        .find(|v| v.id == 4)
        .expect("Visualizer ID 4 (Strings) must exist");
    assert_eq!(freq_vis.name, "Strings");
    assert!(
        matches!(
            freq_vis.pipeline_type,
            crate::state::PipelineType::Mesh3D { .. }
        ),
        "Strings visualizer must use PipelineType::Mesh3D"
    );
}

#[test]
fn test_quick_mix_presets_layout() {
    // Verify that in compact / mobile mode, preset widths and rows never overflow available width
    let mobile_widths = [280.0_f32, 320.0, 360.0, 400.0];
    let spacing = 6.0_f32;

    for &width in &mobile_widths {
        // 2 tracks: 2 buttons per row, 2 rows
        let row_w = ((width - spacing) / 2.0).max(60.0);
        let row_total = row_w * 2.0 + spacing;
        assert!(
            row_total <= width + 0.01,
            "Row width {} must fit in available width {}",
            row_total,
            width
        );

        // 4 tracks: Row 1 has 2 buttons, Row 2 has 2 buttons, Row 3 has 2 buttons
        let chunk_w = ((width - spacing) / 2.0).max(45.0);
        let chunk_total = chunk_w * 2.0 + spacing;
        assert!(
            chunk_total <= width + 0.01,
            "Chunk width {} must fit in available width {}",
            chunk_total,
            width
        );
    }
}

#[test]
fn test_mobile_audio_info_formatting() {
    // Verify mobile portrait single-line audio format string e.g. "Audio: AAC/MP4, X-channels, Y bitrate"
    let format_audio_line = |module_type: &str,
                             codec: Option<&str>,
                             bitrate: Option<u32>,
                             num_channels: usize,
                             tracker_channels: Option<usize>,
                             hardware_channels: usize|
     -> (String, String) {
        let format_name = if let Some(c) = codec {
            if !c.is_empty() && !module_type.eq_ignore_ascii_case(c) {
                format!("{}/{}", c, module_type)
            } else {
                module_type.to_string()
            }
        } else {
            module_type.to_string()
        };

        let ch_info = if let Some(tc) = tracker_channels {
            format!("{} hw / {} tracker", hardware_channels, tc)
        } else if num_channels == 1 {
            "1-channel".to_string()
        } else {
            format!("{}-channels", num_channels)
        };

        let audio_val = if let Some(br) = bitrate {
            format!("{}, {}, {} kbps", format_name, ch_info, br)
        } else {
            format!("{}, {}", format_name, ch_info)
        };

        ("Audio:".to_string(), audio_val)
    };

    // Case 1: AAC in MP4 container with bitrate
    let (label, val) = format_audio_line("MP4", Some("AAC"), Some(256), 2, None, 2);
    assert_eq!(label, "Audio:");
    assert_eq!(val, "AAC/MP4, 2-channels, 256 kbps");

    // Case 2: FLAC, 1-channel with bitrate
    let (_, val) = format_audio_line("FLAC", Some("FLAC"), Some(98), 1, None, 1);
    assert_eq!(val, "FLAC, 1-channel, 98 kbps");

    // Case 3: MP3 with bitrate
    let (_, val) = format_audio_line("MP3", None, Some(320), 2, None, 2);
    assert_eq!(val, "MP3, 2-channels, 320 kbps");

    // Case 4: Tracker module with hardware/tracker channel breakdown (no bitrate)
    let (_, val) = format_audio_line("ProTracker MOD", None, None, 4, Some(4), 4);
    assert_eq!(val, "ProTracker MOD, 4 hw / 4 tracker");

    // Case 5: Composite module_type already containing codec (avoid AAC/AAC/MP4)
    let format_audio_line_dedup = |module_type: &str,
                                   codec: Option<&str>,
                                   bitrate: Option<u32>,
                                   num_channels: usize|
     -> String {
        let format_name = if let Some(c) = codec {
            if !c.is_empty()
                && !module_type
                    .to_ascii_uppercase()
                    .contains(&c.to_ascii_uppercase())
            {
                format!("{}/{}", c, module_type)
            } else {
                module_type.to_string()
            }
        } else {
            module_type.to_string()
        };
        if let Some(br) = bitrate {
            format!("{}, {}-channels, {} kbps", format_name, num_channels, br)
        } else {
            format!("{}, {}-channels", format_name, num_channels)
        }
    };
    let val = format_audio_line_dedup("AAC/MP4", Some("AAC"), Some(788), 2);
    assert_eq!(val, "AAC/MP4, 2-channels, 788 kbps");

    // Case 6: When module_type is "AAC" and file extension is "mp4", container is retained
    let format_audio_with_ext = |format_name: &str, ext: Option<&str>| -> String {
        if let Some(e) = ext {
            let ext_upper = e.to_ascii_uppercase();
            if (ext_upper == "MP4"
                || ext_upper == "M4A"
                || ext_upper == "MOV"
                || ext_upper == "MKV")
                && !format_name.to_ascii_uppercase().contains(&ext_upper)
            {
                format!("{}/{}", format_name, ext_upper)
            } else {
                format_name.to_string()
            }
        } else {
            format_name.to_string()
        }
    };
    assert_eq!(format_audio_with_ext("AAC", Some("mp4")), "AAC/MP4");
    assert_eq!(format_audio_with_ext("AAC/MP4", Some("mp4")), "AAC/MP4");
}

#[test]
fn test_artist_unknown_omission() {
    let should_display_artist = |artist: &str| -> bool {
        let trimmed = artist.trim();
        !trimmed.is_empty()
            && !trimmed.eq_ignore_ascii_case("unknown")
            && !trimmed.eq_ignore_ascii_case("unknown artist")
    };

    assert!(!should_display_artist(""));
    assert!(!should_display_artist("   "));
    assert!(!should_display_artist("Unknown"));
    assert!(!should_display_artist("unknown"));
    assert!(!should_display_artist("UNKNOWN"));
    assert!(!should_display_artist("Unknown Artist"));
    assert!(!should_display_artist("  Unknown  "));
    assert!(should_display_artist("Daft Punk"));
    assert!(should_display_artist("Aphex Twin"));
}

#[test]
fn test_single_open_file_button() {
    let engine_source = include_str!("ui_overlay.rs");
    // Count occurrences of OPEN FILE button definitions in engine.rs
    // 1: In the empty state (no file loaded) -> btn_text = ... "OPEN FILE"
    // 2: In the Track Info header (shared by mobile portrait & desktop) -> "📂 OPEN FILE"
    // Ensure that render_col_info has the only header button and no duplicates exist
    let target_needle = format!("{}(\"{}\")", "egui::RichText::new", "📂 OPEN FILE");
    let rich_open_occurrences = engine_source
        .lines()
        .take_while(|l| !l.contains("mod tests"))
        .filter(|l| l.contains(&target_needle) || l.contains("📂  OPEN FILE"))
        .count();
    assert_eq!(
        rich_open_occurrences, 1,
        "Expected exactly 1 RichText OPEN FILE button in engine.rs Track Info header, found {}",
        rich_open_occurrences
    );
}

#[test]
fn test_fire_visualizers_registration() {
    use std::collections::HashSet;

    // Ensure all visualizer IDs are unique
    let mut ids = HashSet::new();
    for v in crate::state::VISUALIZERS {
        assert!(ids.insert(v.id), "Duplicate visualizer ID found: {}", v.id);
    }

    // Verify Retro Fire (ID 5)
    let retro_fire = crate::state::VISUALIZERS
        .iter()
        .find(|v| v.id == 5)
        .expect("Visualizer ID 5 (Retro Fire) must be registered in VISUALIZERS");
    assert_eq!(retro_fire.name, "Retro Fire");
    assert_eq!(retro_fire.filename, "vis_flame.wgsl");
    assert!(retro_fire.requires_fire, "Retro Fire requires fire compute");

    // Verify Fire Simulation (ID 6)
    let firesim = crate::state::VISUALIZERS
        .iter()
        .find(|v| v.id == 6)
        .expect("Visualizer ID 6 (Fire Simulation) must be registered in VISUALIZERS");
    assert_eq!(firesim.name, "Fire Simulation");
    assert_eq!(firesim.filename, "vis_firesim.wgsl");

    // Verify ID 26 (DOOM Fire) is removed
    assert!(
        crate::state::VISUALIZERS.iter().all(|v| v.id != 26),
        "ID 26 should no longer exist"
    );

    // Verify default enablement states in AppState
    let state = crate::state::AppState::new("Test App".to_string());
    let retro_fire_idx = crate::state::VISUALIZERS
        .iter()
        .position(|v| v.id == 5)
        .unwrap();
    let firesim_idx = crate::state::VISUALIZERS
        .iter()
        .position(|v| v.id == 6)
        .unwrap();
    assert!(
        state.vis_enabled[retro_fire_idx],
        "Retro Fire should be enabled by default"
    );
    assert!(
        !state.vis_enabled[firesim_idx],
        "Fire Simulation should be disabled by default"
    );

    // Verify vis_flame.wgsl contains DOOM palette and CRT post-processing
    let flame_source = include_str!("../../../../src/shaders/vis_flame.wgsl");
    assert!(flame_source.contains("doom_palette"));
    assert!(flame_source.contains("apply_crt_effects"));
}

#[test]
fn test_progress_fire_decay_when_stopped() {
    let mut state = crate::state::AppState::new("Test App".to_string());
    state.file_loaded = true;
    state.is_paused = false;
    state.track_ended = false;

    let mut fire_intensity = 0.0f32;
    let dt = 1.0f32 / 60.0f32;

    // 1. When playing, fire should ramp up to 1.0
    for _ in 0..60 {
        fire_intensity = VulkanEngine::step_fire_intensity(fire_intensity, &state, dt);
    }
    assert!(
        (fire_intensity - 1.0).abs() < 1e-4,
        "Fire should be fully ignited (1.0) while playing"
    );

    // 2. Pause playback -> fire intensity should decay down to 0.0
    state.is_paused = true;
    for _ in 0..60 {
        fire_intensity = VulkanEngine::step_fire_intensity(fire_intensity, &state, dt);
    }
    assert_eq!(
        fire_intensity, 0.0,
        "Fire intensity should die off completely (0.0) when paused"
    );

    // 3. Track ended -> fire intensity should die off
    state.is_paused = false;
    state.track_ended = true;
    fire_intensity = 0.8;
    for _ in 0..60 {
        fire_intensity = VulkanEngine::step_fire_intensity(fire_intensity, &state, dt);
    }
    assert_eq!(
        fire_intensity, 0.0,
        "Fire intensity should die off completely (0.0) when track ended"
    );

    // 4. End of song reached (current_seconds >= duration_seconds) -> fire intensity should die off
    state.track_ended = false;
    state.duration_seconds = 180.0;
    state.current_seconds = 180.0;
    fire_intensity = 0.9;
    for _ in 0..60 {
        fire_intensity = VulkanEngine::step_fire_intensity(fire_intensity, &state, dt);
    }
    assert_eq!(
        fire_intensity, 0.0,
        "Fire intensity should die off completely (0.0) when end of song is reached"
    );
}

#[test]
fn test_retro_fire_audio_reactivity() {
    let mut state = crate::state::AppState::new("Test App".to_string());
    state.file_loaded = true;
    state.is_paused = false;
    state.track_ended = false;

    const W: usize = 320;
    const H: usize = 180;
    let mut cells = [0u8; W * H];
    let mut rng = 0x12345678u32;

    // 1. In silence / quiet passages: hearth is low embers (< 18), flames stay in bottom region
    state.spectrum_data = vec![0.0; 1024];
    state.channel_vus = vec![0.0; 2];
    for _ in 0..40 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 1.0, &state);
    }

    let bottom_row = (H - 1) * W;
    let mut quiet_hearth_sum = 0u32;
    for x in 0..W {
        quiet_hearth_sum += cells[bottom_row + x] as u32;
    }
    let quiet_hearth_avg = quiet_hearth_sum as f32 / W as f32;
    assert!(
        quiet_hearth_avg <= 18.0,
        "Quiet hearth should be low embers (<= 18), got average {:.2}",
        quiet_hearth_avg
    );

    // In quiet passages, flames must not reach upper half of the screen
    let upper_heat_count: usize = cells[..90 * W].iter().filter(|&&c| c > 0).count();
    assert_eq!(
        upper_heat_count, 0,
        "Quiet audio should not produce flames in upper half of screen"
    );

    // 2. On a loud bass kick: center bass columns (x in 140..180) erupt to white-hot (>= 34),
    // and full-spectrum chorus drives the entire hearth to white-hot (>= 33)
    state.spectrum_data[1..16].fill(90.0); // Heavy bass transient
    state.channel_vus = vec![1.0; 2];
    for _ in 0..60 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 1.0, &state);
    }

    let mut bass_col_sum = 0u32;
    let mut loud_hearth_sum = 0u32;
    for x in 0..W {
        let heat = cells[bottom_row + x] as u32;
        loud_hearth_sum += heat;
        if (150..170).contains(&x) {
            bass_col_sum += heat;
        }
    }
    let bass_col_avg = bass_col_sum as f32 / 20.0;
    let loud_hearth_avg = loud_hearth_sum as f32 / W as f32;
    assert!(
        bass_col_avg >= 34.0,
        "Center bass columns should be incandescent white-hot (>= 34), got average {:.2}",
        bass_col_avg
    );
    assert!(
        loud_hearth_avg >= 25.0,
        "Overall hearth on loud beat should be >= 25.0 (quiet was {:.2}), got average {:.2}",
        quiet_hearth_avg,
        loud_hearth_avg
    );

    // Now test full-spectrum chorus
    state.spectrum_data.fill(70.0);
    for _ in 0..60 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 1.0, &state);
    }
    let full_hearth_sum: u32 = (0..W).map(|x| cells[bottom_row + x] as u32).sum();
    let full_hearth_avg = full_hearth_sum as f32 / W as f32;
    assert!(
        full_hearth_avg >= 33.0,
        "Full spectrum chorus hearth should be white-hot (>= 33), got average {:.2}",
        full_hearth_avg
    );

    // Flames must now reach into the upper half of screen
    let loud_upper_heat_count: usize = cells[..90 * W].iter().filter(|&&c| c > 0).count();
    assert!(
        loud_upper_heat_count > 100,
        "Loud beat should push flames well into the upper half of screen, got {} pixels",
        loud_upper_heat_count
    );

    // 3. Stereo channel symmetry: balanced stereo must be symmetric across left/right
    state.tracker_channels = None;
    state.channel_vus = vec![0.8, 0.8];
    for _ in 0..60 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 1.0, &state);
    }
    let mut left_half_sum = 0u32;
    let mut right_half_sum = 0u32;
    for y in 0..H {
        for x in 0..160 {
            left_half_sum += cells[y * W + x] as u32;
        }
        for x in 160..320 {
            right_half_sum += cells[y * W + x] as u32;
        }
    }
    let lr_ratio = right_half_sum as f32 / left_half_sum.max(1) as f32;
    assert!(
        (lr_ratio - 1.0).abs() < 0.05,
        "Balanced stereo must produce symmetric left/right fire energy within 5%, got ratio {:.3} (left: {}, right: {})",
        lr_ratio,
        left_half_sum,
        right_half_sum
    );

    // 4. Stereo panning responsiveness: Left-heavy vs Right-heavy
    state.channel_vus = vec![1.0, 0.1];
    for _ in 0..60 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 1.0, &state);
    }
    let left_heavy_l: u32 = (0..H)
        .flat_map(|y| (0..160).map(move |x| cells[y * W + x] as u32))
        .sum();
    let left_heavy_r: u32 = (0..H)
        .flat_map(|y| (160..320).map(move |x| cells[y * W + x] as u32))
        .sum();
    assert!(
        left_heavy_l > left_heavy_r,
        "Left-panned audio should have more heat on left half (L: {}, R: {})",
        left_heavy_l,
        left_heavy_r
    );

    // 5. Tracker channel separation: Active Track 1 vs Silent Track 2
    state.tracker_channels = Some(4);
    state.spectrum_data = vec![0.0; 1024];
    // [Left_peak, Track1, Track2, Track3, Track4, Right_peak]
    state.channel_vus = vec![0.5, 1.0, 0.0, 0.8, 0.0, 0.5];
    for _ in 0..50 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 1.0, &state);
    }

    // Track 1 column band: x in 0..80. Track 2 column band: x in 80..160
    let mut track1_heat = 0u32;
    let mut track2_heat = 0u32;
    for y in 0..H {
        for x in 0..80 {
            track1_heat += cells[y * W + x] as u32;
        }
        for x in 80..160 {
            track2_heat += cells[y * W + x] as u32;
        }
    }
    assert!(
        track1_heat > track2_heat * 2,
        "Active tracker channel should have vastly higher energy than silent channel (Track1: {}, Track2: {})",
        track1_heat,
        track2_heat
    );

    // 6. Decay to zero when stopped
    for _ in 0..50 {
        VulkanEngine::update_retro_fire_grid(&mut cells, &mut rng, 0.0, &state);
    }
    let remaining_heat: u32 = cells.iter().map(|&c| c as u32).sum();
    assert_eq!(
        remaining_heat, 0,
        "Cells should completely decay to 0 when stopped"
    );
}

#[test]
fn test_windows_installer_icon_registration() {
    let iss_content =
        std::fs::read_to_string(workspace_path("installer.iss")).expect("installer.iss must exist");
    let build_rs =
        std::fs::read_to_string(workspace_path("build.rs")).expect("build.rs must exist");

    // Ensure build.rs explicitly links resource.o / resource.lib into the rusttracker binary
    // so that [lib] in Cargo.toml does not cause the linker to discard the .rsrc section.
    assert!(
        build_rs.contains("cargo:rustc-link-arg-bin=rusttracker={}/resource.o"),
        "build.rs must link resource.o directly into rusttracker.exe on GNU targets"
    );
    assert!(
        build_rs.contains("cargo:rustc-link-arg-bin=rusttracker={}/resource.lib"),
        "build.rs must link resource.lib directly into rusttracker.exe on MSVC targets"
    );

    // Ensure no unquoted or raw .ico,0 references exist (Windows ExtractIconEx fails with .ico,0)
    assert!(
        !iss_content.contains("{app}\\icon.ico,0"),
        "installer.iss must not reference icon.ico,0 as Windows shell requires PE resource indexing on .exe"
    );

    // Ensure ApplicationIcon is registered for the modern Windows 10/11 'Open with' menu
    assert!(
        iss_content.contains(r#"Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: """{app}\rusttracker.exe"",0""#),
        "installer.iss must register ApplicationIcon under Software\\Classes\\Applications\\rusttracker.exe"
    );

    // Ensure DefaultIcon points to the PE resource in rusttracker.exe with quotes
    assert!(
        iss_content.contains(r#"Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"",0""#),
        "installer.iss must register DefaultIcon under Software\\Classes\\Applications\\rusttracker.exe\\DefaultIcon"
    );

    // Ensure App Paths is registered so Windows 11 Open With / AssocQueryString resolves rusttracker.exe
    assert!(
        iss_content.contains(r#"Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\rusttracker.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\rusttracker.exe""#),
        "installer.iss must register App Paths\\rusttracker.exe"
    );

    // Ensure file association ProgIDs have correct quoted executable icon references and Application\ApplicationIcon for Windows 11 OpenWithProgids
    assert!(
        iss_content.contains(r#"Root: HKA; Subkey: "Software\Classes\RustTracker.AudioFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"",0""#),
        "RustTracker.AudioFile must use quoted rusttracker.exe,0 DefaultIcon"
    );
    assert!(
        iss_content.contains(r#"Root: HKA; Subkey: "Software\Classes\RustTracker.AudioFile\Application"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: """{app}\rusttracker.exe"",0""#),
        "RustTracker.AudioFile must register Application\\ApplicationIcon for Windows 11 Open With menu"
    );
}

#[test]
fn test_bitstream_state_reset_on_stream_change() {
    let mut state = crate::state::AppState::new("Test App".to_string());

    // Simulate bitstream playback active
    state.stats.bitstream_active = true;
    state.stats.audio_buffer_fill_pct = 78.5;
    state.stats.video_buffer_fill_pct = 62.0;
    state.stats.video_fps = 59.94;
    state.stats.video_bitrate_kbps = Some(15000);
    state.stats.audio_bitrate_kbps = Some(768);
    state.track_ended = false;
    assert!(state.stats.bitstream_active);
    assert_eq!(state.stats.audio_buffer_fill_pct, 78.5);
    assert_eq!(state.stats.video_fps, 59.94);
    assert_eq!(state.stats.video_bitrate_kbps, Some(15000));
    assert_eq!(state.stats.audio_bitrate_kbps, Some(768));

    // Simulate stream drop / transition logic from main.rs
    state.video_frame_rx = None;
    state.free_video_frame_tx = None;
    state.video_mode = 0;
    state.stats.bitstream_active = false;
    state.stats.audio_buffer_fill_pct = 0.0;
    state.stats.video_buffer_fill_pct = 0.0;
    state.stats.video_hw_decoder.clear();
    state.stats.video_fps = 0.0;
    state.stats.video_bitrate_kbps = None;
    state.stats.audio_bitrate_kbps = None;
    state.track_ended = false;

    assert!(
        !state.stats.bitstream_active,
        "bitstream_active must be false after dropping stream"
    );
    assert_eq!(
        state.stats.audio_buffer_fill_pct, 0.0,
        "audio_buffer_fill_pct must be 0 after dropping stream"
    );
    assert_eq!(
        state.stats.video_buffer_fill_pct, 0.0,
        "video_buffer_fill_pct must be 0 after dropping stream"
    );
    assert_eq!(
        state.stats.video_fps, 0.0,
        "video_fps must be 0 after dropping stream"
    );
    assert!(
        state.stats.video_bitrate_kbps.is_none(),
        "video_bitrate_kbps must be None after dropping stream"
    );
    assert!(
        state.stats.audio_bitrate_kbps.is_none(),
        "audio_bitrate_kbps must be None after dropping stream"
    );
}

#[test]
fn test_push_planar_lookahead_slices_timestamps() {
    let mut state = crate::state::AppState::new("Test App".to_string());
    let sample_rate = 48000;
    let samples = vec![0.5f32; 48000]; // 1 second of audio
    let channels = vec![samples.clone(), samples];
    let current_seconds = 12.5;

    crate::audio::push_planar_lookahead_slices(&mut state, &channels, sample_rate, current_seconds);

    assert!(
        !state.lookahead_queue.is_empty(),
        "Queue must contain slices"
    );
    let (first_t, _) = state.lookahead_queue.front().unwrap();
    let (last_t, _) = state.lookahead_queue.back().unwrap();

    assert!(
        *first_t >= current_seconds,
        "First slice timestamp ({}) must start at or after current_seconds ({})",
        first_t,
        current_seconds
    );
    assert!(
        *last_t > *first_t,
        "Last slice timestamp ({}) must be ahead of first slice timestamp ({})",
        last_t,
        first_t
    );
    assert!(
        (*last_t - current_seconds) >= 0.9,
        "Lookahead span should extend ~1s into the future"
    );
}

#[test]
fn test_lookahead_timeline_continuity_and_synthesis() {
    let mut state = crate::state::AppState::new("Test App".to_string());
    state.lookahead_timeline = vec![0.0; 600 * 8];

    // Part 1: Full 7-second decoded lookahead buffer (normal playback)
    for i in 0..700 {
        let t = 5.0 + (i as f64 * 0.01);
        let s = (i as f32 * 0.05).sin() * 0.4;
        state.lookahead_queue.push_back((
            t,
            [-s.abs(), s.abs(), -s.abs(), s.abs(), 0.35, 0.35, 0.5, 0.5],
        ));
    }

    let cur_t = 6.0;
    let q_len = state.lookahead_queue.len();
    let (t_first, d_first) = state.lookahead_queue[0];
    let (t_last, d_last) = state.lookahead_queue[q_len - 1];

    for k in 0..600 {
        let target_t = cur_t + (k as f64 - 100.0) * 0.01;
        let off = k * 8;

        if target_t < t_first {
            let fade = (1.0 - ((t_first - target_t) / 0.5)).max(0.0) as f32;
            for (c, val) in d_first.iter().enumerate() {
                state.lookahead_timeline[off + c] = *val * fade;
            }
        } else if target_t > t_last {
            let fade = (1.0 - ((target_t - t_last) / 0.5)).max(0.0) as f32;
            for (c, val) in d_last.iter().enumerate() {
                state.lookahead_timeline[off + c] = *val * fade;
            }
        } else {
            let idx = match state.lookahead_queue.binary_search_by(|(t, _)| {
                t.partial_cmp(&target_t)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                Ok(i) => i,
                Err(i) => i,
            };

            if idx == 0 {
                state.lookahead_timeline[off..off + 8].copy_from_slice(&d_first);
            } else if idx >= q_len {
                state.lookahead_timeline[off..off + 8].copy_from_slice(&d_last);
            } else {
                let (t0, d0) = state.lookahead_queue[idx - 1];
                let (t1, d1) = state.lookahead_queue[idx];
                let dt = t1 - t0;
                if dt > 0.0001 && dt < 1.0 && target_t >= t0 && target_t <= t1 {
                    let frac = ((target_t - t0) / dt).clamp(0.0, 1.0) as f32;
                    for c in 0..8 {
                        state.lookahead_timeline[off + c] = d0[c] * (1.0 - frac) + d1[c] * frac;
                    }
                } else {
                    state.lookahead_timeline[off..off + 8].copy_from_slice(&d0);
                }
            }
        }
    }

    // Verify that lookahead is continuous with NO step discontinuities / banding across adjacent slices
    for k in 0..599 {
        let off0 = k * 8;
        let off1 = (k + 1) * 8;
        let max0 = state.lookahead_timeline[off0 + 1];
        let max1 = state.lookahead_timeline[off1 + 1];
        let diff = (max1 - max0).abs();
        assert!(
            diff < 0.05,
            "Adjacent slices {} and {} must be continuous (diff: {})",
            k,
            k + 1,
            diff
        );
    }

    // Part 2: Fallback synthesis when queue is empty
    state.lookahead_queue.clear();
    state.raw_waveform = vec![0.5; 512];
    state.raw_channel_vus = vec![0.7, 0.7];

    let wave_len = state.raw_waveform.len();
    let vu_l = state
        .raw_channel_vus
        .first()
        .copied()
        .unwrap_or(0.2)
        .clamp(0.05, 1.0);
    let vu_r = state
        .raw_channel_vus
        .get(1)
        .copied()
        .unwrap_or(vu_l)
        .clamp(0.05, 1.0);
    for k in 0..600 {
        let off = k * 8;
        let wave_idx = (k * 13) % wave_len;
        let sample = state.raw_waveform[wave_idx].clamp(-1.0, 1.0);
        let sample_next = state.raw_waveform[(wave_idx + 1) % wave_len].clamp(-1.0, 1.0);
        let min_s = sample.min(sample_next).min(0.0) * vu_l;
        let max_s = sample.max(sample_next).max(0.0) * vu_l;
        let min_r = sample.min(sample_next).min(0.0) * vu_r;
        let max_r = sample.max(sample_next).max(0.0) * vu_r;
        let rms = (sample.abs() * 0.7 + 0.3 * (vu_l + vu_r) * 0.5).min(1.0);
        state.lookahead_timeline[off] = min_s;
        state.lookahead_timeline[off + 1] = max_s;
        state.lookahead_timeline[off + 2] = min_r;
        state.lookahead_timeline[off + 3] = max_r;
        state.lookahead_timeline[off + 4] = rms;
        state.lookahead_timeline[off + 5] = rms;
        state.lookahead_timeline[off + 6] = vu_l;
        state.lookahead_timeline[off + 7] = vu_r;
    }

    for k in 0..600 {
        let off = k * 8;
        let max_s = state.lookahead_timeline[off + 1];
        let rms = state.lookahead_timeline[off + 4];
        assert!(max_s > 0.0, "Fallback slice {} max_s should be positive", k);
        assert!(rms > 0.0, "Fallback slice {} rms should be non-zero", k);
    }
}

#[test]
fn test_open_file_button_theme_colors() {
    let prod_code = engine_prod_source();

    // Assert no old cyan/blue button colors in empty state or header button
    assert!(
        !prod_code.contains("Color32::from_rgb(0, 150, 220)"),
        "Button must not use cyan/blue fill color"
    );
    assert!(
        !prod_code.contains("Color32::from_rgb(0, 185, 255)"),
        "Button must not use bright blue hover fill color"
    );
    assert!(
        !prod_code.contains("Color32::from_rgb(100, 220, 255)"),
        "Button must not use cyan stroke color"
    );

    // Assert signature orange/amber theme colors are present
    assert!(
        prod_code.contains("Color32::from_rgb(205, 95, 0)"),
        "Button must use theme amber/orange fill color"
    );
    assert!(
        prod_code.contains("Color32::from_rgb(255, 145, 30)"),
        "Button must use theme orange stroke color"
    );
}

#[test]
fn test_macos_bundle_and_entitlements_config() {
    let macos_yml = std::fs::read_to_string(workspace_path(".github/workflows/macos.yml"))
        .expect(".github/workflows/macos.yml must exist")
        .replace("\r\n", "\n");
    let entitlements = std::fs::read_to_string(workspace_path("entitlements.plist"))
        .expect("entitlements.plist must exist");

    // Verify required microphone & library entitlements are present
    assert!(
        entitlements.contains("com.apple.security.device.audio-input"),
        "entitlements.plist must request audio-input for --mic support"
    );
    assert!(
        entitlements.contains("com.apple.security.cs.disable-library-validation"),
        "entitlements.plist must allow loading bundled Homebrew dylibs"
    );

    // Verify both Developer ID and ad-hoc signing pass --entitlements entitlements.plist
    assert!(
        macos_yml.contains(
            "codesign --force --entitlements entitlements.plist --sign - RustTracker.app/Contents/MacOS/rusttracker"
        ),
        "Ad-hoc signing must embed entitlements.plist on the main executable"
    );
    assert!(
        macos_yml.contains(
            "codesign --force --entitlements entitlements.plist --sign - RustTracker.app"
        ),
        "Ad-hoc signing must embed entitlements.plist on the app bundle"
    );

    // Verify minimum macOS version matches Apple Silicon baseline (11.0)
    assert!(
        macos_yml
            .contains("<key>LSMinimumSystemVersion</key>\n              <string>11.0</string>"),
        "Info.plist LSMinimumSystemVersion must be 11.0 for Apple Silicon builds"
    );

    // Verify every extension in FILE_DIALOG_EXTENSIONS is registered in Info.plist CFBundleDocumentTypes
    for ext in crate::playlist::FILE_DIALOG_EXTENSIONS {
        let tag = format!("<string>{}</string>", ext);
        assert!(
            macos_yml.contains(&tag),
            "macOS Info.plist CFBundleDocumentTypes is missing extension: {}",
            ext
        );
    }
}

#[test]
fn test_app_default_grey_background_invariants() {
    // 1. Verify the canonical application default grey clear color constant
    assert_eq!(
        APP_DEFAULT_BG_CLEAR_COLOR,
        wgpu::Color {
            r: 0.1,
            g: 0.1,
            b: 0.1,
            a: 1.0,
        },
        "APP_DEFAULT_BG_CLEAR_COLOR must remain (0.1, 0.1, 0.1, 1.0) so the HUD/UI panel background stays default grey"
    );

    let prod_code = engine_prod_source();

    // 2. Main Render Pass must clear to APP_DEFAULT_BG_CLEAR_COLOR and never inline a custom wgpu::Color
    assert!(
        prod_code.contains("load: wgpu::LoadOp::Clear(APP_DEFAULT_BG_CLEAR_COLOR)"),
        "Main Render Pass must clear using APP_DEFAULT_BG_CLEAR_COLOR"
    );
    assert!(
        !prod_code.contains("LoadOp::Clear(wgpu::Color"),
        "Main Render Pass must not override APP_DEFAULT_BG_CLEAR_COLOR with an inline wgpu::Color"
    );
}

#[test]
fn test_visualizer_curation_and_glass_water_overhaul() {
    // 1. Verify retired visualizers 8 (Spatial Vectors), 9 (Chrome Ferrofluid), 10 (Ferrofluid Particle Sim),
    // 22 (Neon Spatial Room), and 30 (Acoustic Wave Chamber) are removed
    for retired_id in [8u32, 9, 10, 22, 30] {
        assert!(
            crate::state::VISUALIZERS.iter().all(|v| v.id != retired_id),
            "Visualizer ID {} must be removed from VISUALIZERS",
            retired_id
        );
    }

    // 2. Verify Acoustic Glass Water Chamber (ID 28) remains registered and honestly analytic
    let glass_water = crate::state::VISUALIZERS
        .iter()
        .find(|v| v.id == 28)
        .expect("Visualizer ID 28 (Acoustic Glass Water Chamber) must be registered");
    assert_eq!(glass_water.filename, "vis_glass_water.wgsl");
    assert!(!glass_water.requires_flip_water);

    // 3. Validate WGSL compilation and key optical/analytic wave features of vis_glass_water.wgsl
    const SHADER_COMMON: &str = include_str!("../../../../src/shaders/_common.wgsl");
    let glass_src = include_str!("../../../../src/shaders/vis_glass_water.wgsl")
        .replace("// INCLUDE: common", SHADER_COMMON);
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&glass_src) {
        panic!(
            "vis_glass_water.wgsl WGSL parse error:\n{}",
            e.emit_to_string(&glass_src)
        );
    }
    assert!(!glass_src.contains("eval_cylinder_wall_glass"));
    assert!(glass_src.contains("eval_studio_reflection"));
    assert!(glass_src.contains("sample_macro_acoustic_energy"));

    // 4. Validate WGSL parsing for all optimized visualizers (Tape Head, LED Spectrum, Canyon, Neon Room, Synthwave, Biolum, Ferrofluid)
    const SHADER_GLYPH_FONT: &str = include_str!("../../../../src/shaders/_glyph_font.wgsl");
    for (name, raw_src) in [
        (
            "vis_tape_head.wgsl",
            include_str!("../../../../src/shaders/vis_tape_head.wgsl"),
        ),
        (
            "vis_spectrum_led.wgsl",
            include_str!("../../../../src/shaders/vis_spectrum_led.wgsl"),
        ),
        (
            "vis_spectrum_canyon.wgsl",
            include_str!("../../../../src/shaders/vis_spectrum_canyon.wgsl"),
        ),
        (
            "vis_neon_acoustic_room.wgsl",
            include_str!("../../../../src/shaders/vis_neon_acoustic_room.wgsl"),
        ),
        (
            "vis_synthwave.wgsl",
            include_str!("../../../../src/shaders/vis_synthwave.wgsl"),
        ),
        (
            "vis_synthwave_racer_3d.wgsl",
            include_str!("../../../../src/shaders/vis_synthwave_racer_3d.wgsl"),
        ),
        (
            "vis_synthwave_racer_sky.wgsl",
            include_str!("../../../../src/shaders/vis_synthwave_racer_sky.wgsl"),
        ),
        (
            "vis_bioluminescence.wgsl",
            include_str!("../../../../src/shaders/vis_bioluminescence.wgsl"),
        ),
        (
            "biolum_compute.wgsl",
            include_str!("../../../../src/shaders/biolum_compute.wgsl"),
        ),
        (
            "vis_flip_ferrofluid.wgsl",
            include_str!("../../../../src/shaders/vis_flip_ferrofluid.wgsl"),
        ),
        (
            "flip_ferrofluid_compute.wgsl",
            include_str!("../../../../src/shaders/flip_ferrofluid_compute.wgsl"),
        ),
        (
            "vis_vumeters_bg.wgsl",
            include_str!("../../../../src/shaders/vis_vumeters_bg.wgsl"),
        ),
    ] {
        let resolved = raw_src
            .replace("// INCLUDE: common", SHADER_COMMON)
            .replace("// INCLUDE: glyph_font", SHADER_GLYPH_FONT);
        if let Err(e) = wgpu::naga::front::wgsl::parse_str(&resolved) {
            panic!(
                "{} WGSL parse error:\n{}",
                name,
                e.emit_to_string(&resolved)
            );
        }
    }
}

#[test]
fn test_hardware_capability_tier_and_vsync_stats_panel() {
    use crate::state::{GpuCapabilityTier, VISUALIZERS, classify_gpu_capability};

    // 1. Verify crates/rt-core/src/state.rs contains zero OS-based (`target_os = "android"`) visualizer exclusions
    let state_code = std::fs::read_to_string(workspace_path("crates/rt-core/src/state.rs"))
        .expect("crates/rt-core/src/state.rs must exist");
    assert!(
        !state_code.contains("target_os = \"android\""),
        "Visualizers must never be disabled based on target_os = \"android\""
    );
    assert_eq!(
        VISUALIZERS.len(),
        19,
        "All 19 visualizers must be compiled on every platform"
    );

    let make_adapter = |name: &str, device_type: wgpu::DeviceType| wgpu::AdapterInfo {
        name: name.to_string(),
        vendor: 0,
        device: 0,
        device_type,
        device_pci_bus_id: String::new(),
        driver: String::new(),
        driver_info: String::new(),
        backend: wgpu::Backend::Vulkan,
        subgroup_min_size: 32,
        subgroup_max_size: 64,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    };

    // 2. Flagship Android phone GPUs and discrete desktop GPUs -> GpuCapabilityTier::High
    for (gpu_name, dev_type, w, h) in [
        (
            "Adreno (TM) 740",
            wgpu::DeviceType::IntegratedGpu,
            3088,
            1440,
        ),
        (
            "Adreno (TM) 750",
            wgpu::DeviceType::IntegratedGpu,
            3120,
            1440,
        ),
        (
            "Adreno (TM) 830",
            wgpu::DeviceType::IntegratedGpu,
            3200,
            1440,
        ),
        (
            "Immortalis-G720 MC12",
            wgpu::DeviceType::IntegratedGpu,
            2800,
            1260,
        ),
        (
            "Samsung Xclipse 940",
            wgpu::DeviceType::IntegratedGpu,
            3120,
            1440,
        ),
        ("Apple M3 Max", wgpu::DeviceType::IntegratedGpu, 3456, 2234),
        (
            "AMD Radeon AI PRO R9700",
            wgpu::DeviceType::DiscreteGpu,
            3840,
            2160,
        ),
    ] {
        let info = make_adapter(gpu_name, dev_type);
        let profile = gpu_adapter_profile_from_wgpu(&info);
        assert_eq!(
            classify_gpu_capability(&profile, w * h),
            GpuCapabilityTier::High,
            "{} should classify as High capability tier",
            gpu_name
        );
        let mut state = AppState::new("Test".to_string());
        state.apply_gpu_capability_profile(&profile, w, h);
        assert_eq!(state.gpu_capability_tier, GpuCapabilityTier::High);
        for (idx, v) in VISUALIZERS.iter().enumerate() {
            if !matches!(v.id, 6 | 7 | 23) {
                assert!(
                    state.vis_enabled[idx],
                    "Visualizer {} ({}) must remain enabled on High-tier GPU {}",
                    v.id, v.name, gpu_name
                );
            }
        }
    }

    // 3. Constrained GPUs (Pixel Tablet Mali-G710 @ 2560x1600, mid-range Adreno 640, Steam Deck APU) -> Constrained
    for (gpu_name, dev_type, w, h) in [
        ("Mali-G710", wgpu::DeviceType::IntegratedGpu, 2560, 1600),
        (
            "Adreno (TM) 640",
            wgpu::DeviceType::IntegratedGpu,
            2400,
            1080,
        ),
        (
            "AMD Custom GPU 0405 (RADV VANGOGH)",
            wgpu::DeviceType::IntegratedGpu,
            1280,
            800,
        ),
        (
            "llvmpipe (LLVM 19.1.0, 256 bits)",
            wgpu::DeviceType::Cpu,
            1920,
            1080,
        ),
    ] {
        let info = make_adapter(gpu_name, dev_type);
        let profile = gpu_adapter_profile_from_wgpu(&info);
        assert_eq!(
            classify_gpu_capability(&profile, w * h),
            GpuCapabilityTier::Constrained,
            "{} should classify as Constrained capability tier",
            gpu_name
        );
        let mut state = AppState::new("Test".to_string());
        let mut default_off = AppState::new("Default".to_string());
        default_off.apply_gpu_capability_profile(&profile, w, h);
        assert!(
            default_off
                .vis_enabled
                .iter()
                .zip(AppState::new("Ref".to_string()).vis_enabled.iter())
                .all(|(a, b)| a == b),
            "Constrained GPU must not auto-uncheck visualizers without --auto-vis-guard ({gpu_name})"
        );
        state.auto_vis_guard = true;
        state.apply_gpu_capability_profile(&profile, w, h);
        assert_eq!(state.gpu_capability_tier, GpuCapabilityTier::Constrained);
        for (idx, v) in VISUALIZERS.iter().enumerate() {
            if v.heavy_shader {
                assert!(
                    !state.vis_enabled[idx],
                    "Heavy shader visualizer {} ({}) should default to unchecked on Constrained GPU {}",
                    v.id, v.name, gpu_name
                );
            } else {
                assert!(
                    state.vis_enabled[idx],
                    "Standard visualizer {} ({}) must remain enabled on Constrained GPU {}",
                    v.id, v.name, gpu_name
                );
            }
        }
    }

    // 4. Runtime Frame-Budget Watchdog auto-disables non-overridden heavy shader if < 85% VSync FPS
    // Default (no --auto-vis-guard): watchdog warns via OSD but never switches or unchecks
    let mut passive = AppState::new("Passive".to_string());
    passive.file_loaded = true;
    passive.is_paused = false;
    let passive_idx = VISUALIZERS
        .iter()
        .position(|v| v.id == 28)
        .expect("ID 28 must exist");
    passive.current_visualizer_idx = passive_idx;
    passive.visualizer_mode = 28;
    for _ in 0..300 {
        passive.update_visualizer_perf_guard(1.0 / 30.0, 60.0);
    }
    assert!(passive.vis_enabled[passive_idx]);
    assert_eq!(passive.current_visualizer_idx, passive_idx);
    assert!(
        passive
            .osd_text
            .as_deref()
            .is_some_and(|t| t.contains("--auto-vis-guard"))
    );

    let mut state = AppState::new("Watchdog Test".to_string());
    state.auto_vis_guard = true;
    state.file_loaded = true;
    state.is_paused = false;
    let heavy_idx = VISUALIZERS
        .iter()
        .position(|v| v.id == 28)
        .expect("ID 28 must exist");
    state.current_visualizer_idx = heavy_idx;
    state.visualizer_mode = 28;
    for _ in 0..140 {
        // Simulate 33.3ms frame time (30 FPS on a 60Hz display)
        state.update_visualizer_perf_guard(1.0 / 30.0, 60.0);
    }
    assert!(
        !state.vis_enabled[heavy_idx],
        "Watchdog must uncheck heavy visualizer when it drops to 30 FPS on 60Hz display"
    );
    assert_ne!(
        state.current_visualizer_idx, heavy_idx,
        "Watchdog must advance to the next enabled visualizer"
    );

    // 5. Explicit user selection (`vis_user_overridden`) bypasses the watchdog
    let mut user_state = AppState::new("User Override Test".to_string());
    user_state.auto_vis_guard = true;
    user_state.file_loaded = true;
    user_state.is_paused = false;
    user_state.current_visualizer_idx = heavy_idx;
    user_state.visualizer_mode = 28;
    user_state.vis_user_overridden[heavy_idx] = true;
    for _ in 0..200 {
        user_state.update_visualizer_perf_guard(1.0 / 25.0, 60.0);
    }
    assert!(
        user_state.vis_enabled[heavy_idx],
        "User-overridden visualizer must never be auto-disabled by the watchdog"
    );
    assert_eq!(user_state.current_visualizer_idx, heavy_idx);

    // 6. Verify VSYNC status formatting and Stats panel integration
    assert_eq!(format_vsync_status(wgpu::PresentMode::Fifo), "VSYNC: ON");
    assert_eq!(
        format_vsync_status(wgpu::PresentMode::FifoRelaxed),
        "VSYNC: ON"
    );
    assert_eq!(
        format_vsync_status(wgpu::PresentMode::AutoVsync),
        "VSYNC: ON"
    );
    assert_eq!(
        format_vsync_status(wgpu::PresentMode::AutoNoVsync),
        "VSYNC: OFF"
    );
    assert_eq!(
        format_vsync_status(wgpu::PresentMode::Immediate),
        "VSYNC: OFF"
    );
    assert_eq!(
        format_vsync_status(wgpu::PresentMode::Mailbox),
        "VSYNC: OFF"
    );

    let prod_code = engine_prod_source();
    assert!(
        prod_code.contains("format_vsync_status(self.config.present_mode)"),
        "Stats panel must display VSYNC: ON / VSYNC: OFF status"
    );
}

#[test]
fn test_appimage_packaging_and_apprun_config() {
    let repackage = std::fs::read_to_string(workspace_path("repackage_appimage.sh"))
        .expect("repackage_appimage.sh must exist");
    let apprun = std::fs::read_to_string(workspace_path("AppRun.template"))
        .expect("AppRun.template must exist");
    let linux_yml = std::fs::read_to_string(workspace_path(".github/workflows/linux-build.yml"))
        .expect(".github/workflows/linux-build.yml must exist");

    // 1. Verify modern AppImage/appimagetool repository URL (not deprecated AppImage/AppImageKit)
    assert!(
        repackage.contains("https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"),
        "repackage_appimage.sh must fetch appimagetool from AppImage/appimagetool"
    );
    assert!(
        !repackage.contains("AppImage/AppImageKit"),
        "repackage_appimage.sh must not reference deprecated AppImage/AppImageKit URL"
    );

    // 2. Verify rDNS AppStream metadata alignment for zero-warning appstreamcli validation
    assert!(
        repackage.contains("com.soddentrough.rusttracker.appdata.xml"),
        "repackage_appimage.sh must install com.soddentrough.rusttracker.appdata.xml"
    );
    assert!(
        repackage.contains("com.soddentrough.rusttracker.desktop"),
        "repackage_appimage.sh must align desktop entry filename to rDNS ID"
    );

    // 3. Verify AppRun.template preserves XDG_DATA_DIRS fallback and configures Gamescope Wayland
    assert!(
        apprun.contains("/usr/local/share:/usr/share"),
        "AppRun.template must preserve standard XDG_DATA_DIRS fallback paths"
    );
    assert!(
        apprun.contains("RUSTTRACKER_WAYLAND"),
        "AppRun.template must export RUSTTRACKER_WAYLAND under Gamescope / Steam Deck"
    );

    // 4. Verify Linux CI workflow packages and uploads the AppImage
    assert!(
        linux_yml.contains("RustTracker-Linux-AppImage"),
        "linux-build.yml must upload the RustTracker-Linux-AppImage artifact"
    );
    assert!(
        linux_yml.contains("RustTracker-SteamDeck-*.AppImage"),
        "linux-build.yml must publish RustTracker-SteamDeck-*.AppImage on release tags"
    );
}

#[test]
fn test_splash_screen_cache_and_holistic_ux() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0));
    let mut splash_cache = SplashCache::default();

    // 1. Static background mesh should build once and remain cached for identical viewport geometry
    splash_cache.ensure_background(&ctx, rect);
    assert!(
        !splash_cache.static_bg_mesh.vertices.is_empty(),
        "Static splash background mesh must contain tessellated sky, sun, mountains, and perspective grid"
    );
    assert_eq!(
        splash_cache.stars.len(),
        56,
        "SplashCache must precompute 56 stars"
    );
    let first_arc_ptr = std::sync::Arc::as_ptr(&splash_cache.static_bg_mesh);

    splash_cache.ensure_background(&ctx, rect);
    assert_eq!(
        std::sync::Arc::as_ptr(&splash_cache.static_bg_mesh),
        first_arc_ptr,
        "SplashCache must reuse the cached Arc<Mesh> when viewport dimensions and ppp do not change"
    );

    // 2. Dynamic background mesh builds twinkling stars + scrolling horizontal grid lines in a single Mesh
    let dyn_mesh = splash_cache.build_dynamic_bg_mesh(rect, 1.25, 2.0);
    assert!(
        dyn_mesh.vertices.len() >= 50 * 4,
        "Dynamic splash mesh must contain at least 50 star quads plus scrolling grid lines"
    );

    // 2b. Title chrome mesh builds 24 slices over tight glyph bounds with C0 slice-boundary continuity
    // and anchored sky->white-horizon->obsidian-crease->sunset-gold reflection (zero barber-pole wrap seam).
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Orbitron".to_owned(),
        egui::FontData::from_static(EMBEDDED_GLASS_FONT).into(),
    );
    fonts.families.insert(
        egui::FontFamily::Name("Orbitron".into()),
        vec!["Orbitron".to_owned()],
    );
    ctx.set_fonts(fonts);

    let title_rect = egui::Rect::from_center_size(egui::pos2(640.0, 90.0), egui::vec2(550.0, 70.0));
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            splash_cache.update_title_mesh(ui, title_rect, 29.0, 66.0, 1.25);
        });
    });
    output.textures_delta.clear();
    assert_eq!(
        splash_cache.title_slice_ranges.len(),
        24,
        "Splash title mesh must use 24 vertical chrome slices"
    );
    assert!(
        splash_cache.title_glyph_bounds.width() > 100.0
            && splash_cache.title_glyph_bounds.height() > 20.0,
        "Splash title must compute tight glyph bounding box"
    );

    // Verify adjacent slices share identical vertex colors at their shared horizontal seam (y_bot == y_top)
    let mut matched_seams = 0usize;
    for w in splash_cache.title_slice_ranges.windows(2) {
        let (s0, e0) = w[0];
        let (s1, e1) = w[1];
        let quads0 = splash_cache.title_base_mesh.vertices[s0..e0]
            .as_chunks::<4>()
            .0;
        let quads1 = splash_cache.title_base_mesh.vertices[s1..e1]
            .as_chunks::<4>()
            .0;
        for q0 in quads0 {
            for q1 in quads1 {
                if (q0[0].pos.x - q1[0].pos.x).abs() < 1e-3
                    && (q0[2].pos.y - q1[0].pos.y).abs() < 1e-3
                {
                    assert_eq!(
                        q0[3].color, q1[0].color,
                        "Left vertex color at slice boundary must be C0 continuous"
                    );
                    assert_eq!(
                        q0[2].color, q1[1].color,
                        "Right vertex color at slice boundary must be C0 continuous"
                    );
                    matched_seams += 1;
                }
            }
        }
    }
    assert!(
        matched_seams >= 100,
        "All active glyph slice boundaries must match seamlessly"
    );

    // Verify chrome reflection remains anchored (sky on top, warm sunset on bottom) at all times
    for &sample_t in &[0.0_f32, 1.7, 3.4, 5.1, 8.8] {
        let top_col = SplashCache::evaluate_chrome_color(0.5, 0.02, 0.5, sample_t);
        let bot_col = SplashCache::evaluate_chrome_color(0.5, 0.98, 0.5, sample_t);
        assert!(
            top_col.b() > top_col.r(),
            "Top of chrome glyphs must reflect blue/cyan sky at t={sample_t}"
        );
        assert!(
            bot_col.r() > bot_col.b(),
            "Bottom of chrome glyphs must reflect warm sunset coral/gold at t={sample_t}"
        );
    }

    // 3. StatsDisplayCache throttles string formatting to 10 Hz (100ms) and separates CPU command recording from driver submit+present
    let mut stats_cache = StatsDisplayCache::default();
    let mut state = AppState::new("Stats Cache Test".to_string());
    state.current_fps = 60.0;
    state.stats.phase_lock_update_us = 100.0;
    state.stats.phase_snapshot_us = 20.0;
    state.stats.phase_surface_us = 13500.0;
    state.stats.phase_egui_layout_us = 1200.0;
    state.stats.phase_encode_us = 200.0;
    state.stats.phase_submit_us = 1400.0;
    state.stats.phase_post_us = 10.0;
    stats_cache.update_if_needed(&state, false, 1.0);
    assert_eq!(stats_cache.fps_line, "FPS: 60.0 (16.67 ms)");
    assert_eq!(
        stats_cache.cpu_active_line, "CPU Active Total: 1.53 ms",
        "CPU Active Total must sum only active CPU phases (excluding VSync wait and Submit+Present)"
    );
    assert_eq!(stats_cache.phase_lines[3].0, "  UI Layout + Tess: 1.20 ms");
    assert_eq!(stats_cache.phase_lines[4].0, "  Cmd Record (CPU): 0.20 ms");
    assert_eq!(stats_cache.phase_lines[5].0, "  Submit + Present: 1.40 ms");
    assert_eq!(
        stats_cache.total_phases_line,
        "  Total Frame Loop: 16.43 ms"
    );

    // Within 100ms window (t = 1.03s), cached strings must remain stable even if raw telemetry fluctuates
    state.current_fps = 50.0;
    stats_cache.update_if_needed(&state, false, 1.03);
    assert_eq!(
        stats_cache.fps_line, "FPS: 60.0 (16.67 ms)",
        "StatsDisplayCache must throttle updates within the 100ms window"
    );

    // After >= 100ms (t = 1.12s), cached strings must refresh
    stats_cache.update_if_needed(&state, false, 1.12);
    assert_eq!(
        stats_cache.fps_line, "FPS: 50.0 (20.00 ms)",
        "StatsDisplayCache must refresh after 100ms"
    );
}

#[test]
fn test_vu_meter_phosphor_ghosting_decay() {
    // 1. Validate hud.wgsl compiles cleanly with naga
    const SHADER_COMMON: &str = include_str!("../../../../src/shaders/_common.wgsl");
    let hud_resolved = include_str!("../../../../src/shaders/hud.wgsl")
        .replace("// INCLUDE: common", SHADER_COMMON);
    if let Err(e) = wgpu::naga::front::wgsl::parse_str(&hud_resolved) {
        panic!(
            "hud.wgsl WGSL parse error:\n{}",
            e.emit_to_string(&hud_resolved)
        );
    }

    let mut prev_active = [0u32; 32];
    let mut segment_glows = [[0.0f32; 30]; 32];
    let mut channels = [0.0f32; 32];

    let unpack_seg = |packed: &[u32; 256], ch: usize, seg: usize| -> f32 {
        let word = packed[ch * 8 + (seg >> 2)];
        let byte = (word >> ((seg & 3) * 8)) & 0xFF;
        byte as f32 / 255.0
    };

    // 2. Rising from 0 to 20 segments must NOT trigger ghosting above the bar
    channels[0] = 20.0 / 30.0;
    let packed = VulkanEngine::step_vu_phosphor_ghosts(
        &mut prev_active,
        &mut segment_glows,
        &channels,
        2,
        0.010,
    );
    assert_eq!(prev_active[0], 20);
    for s in 0..30 {
        assert_eq!(
            unpack_seg(&packed, 0, s),
            0.0,
            "Rising VU bar must not light unlit blocks"
        );
    }

    // 3. Drop by 4 blocks (20 -> 16): segments 16..20 ignite at 1.0, segments 20..30 stay 0.0
    channels[0] = 16.0 / 30.0;
    let packed = VulkanEngine::step_vu_phosphor_ghosts(
        &mut prev_active,
        &mut segment_glows,
        &channels,
        2,
        0.010,
    );
    assert_eq!(prev_active[0], 16);
    for s in 16..20 {
        assert!(
            (unpack_seg(&packed, 0, s) - 1.0).abs() < 1e-3,
            "Deactivated segment {} should start fade at 1.0",
            s
        );
    }
    assert_eq!(unpack_seg(&packed, 0, 20), 0.0);

    // 4. Advance by half the fade duration (175 ms of 350 ms): segments 16..20 should be at ~0.50
    let half_dt = VulkanEngine::VU_PHOSPHOR_FADE_SECS * 0.5;
    let packed = VulkanEngine::step_vu_phosphor_ghosts(
        &mut prev_active,
        &mut segment_glows,
        &channels,
        2,
        0.100,
    );
    let _ = packed;
    let packed = VulkanEngine::step_vu_phosphor_ghosts(
        &mut prev_active,
        &mut segment_glows,
        &channels,
        2,
        half_dt - 0.100,
    );
    for s in 16..20 {
        let g = unpack_seg(&packed, 0, s);
        assert!(
            (g - 0.50).abs() < 0.02,
            "At half fade duration, segment {} should be ~0.50 (got {})",
            s,
            g
        );
    }

    // 5. Partial upward bounce (16 -> 18) must NOT wipe out still-fading segments 18..20 above it!
    channels[0] = 18.0 / 30.0;
    let packed = VulkanEngine::step_vu_phosphor_ghosts(
        &mut prev_active,
        &mut segment_glows,
        &channels,
        2,
        0.035,
    );
    assert_eq!(prev_active[0], 18);
    assert_eq!(
        unpack_seg(&packed, 0, 17),
        0.0,
        "Active segment 17 is drawn via is_active"
    );
    assert!(
        (unpack_seg(&packed, 0, 18) - 0.40).abs() < 0.02,
        "Segment 18 above bounce must continue fading smoothly (got {})",
        unpack_seg(&packed, 0, 18)
    );
    assert!(
        (unpack_seg(&packed, 0, 19) - 0.40).abs() < 0.02,
        "Segment 19 above bounce must continue fading smoothly (got {})",
        unpack_seg(&packed, 0, 19)
    );

    // 6. Advance remaining fade time: segments 18..20 reach 0.0
    for _ in 0..3 {
        let _ = VulkanEngine::step_vu_phosphor_ghosts(
            &mut prev_active,
            &mut segment_glows,
            &channels,
            2,
            0.080,
        );
    }
    let packed = VulkanEngine::step_vu_phosphor_ghosts(
        &mut prev_active,
        &mut segment_glows,
        &channels,
        2,
        0.010,
    );
    assert!(unpack_seg(&packed, 0, 18) < 1e-4);
    assert!(unpack_seg(&packed, 0, 19) < 1e-4);
}

#[test]
fn test_file_transfer_state_progress_and_snapshot() {
    let mut state = AppState::new("Test".to_string());
    assert!(state.file_transfer.is_none());

    // Indeterminate phase (cloud download inside openInputStream before bytes stream)
    state.file_transfer = Some(crate::state::FileTransferState {
        file_name: "Atmos_Demo.mkv".to_string(),
        status_text: "Downloading from Google Drive...".to_string(),
        bytes_copied: 0,
        total_bytes: Some(50 * 1024 * 1024),
        elapsed_secs: 1.5,
    });
    let snap1 = state.render_snapshot();
    let ft1 = snap1
        .file_transfer
        .as_ref()
        .expect("render_snapshot must preserve active file_transfer");
    assert_eq!(ft1.file_name, "Atmos_Demo.mkv");
    assert_eq!(ft1.progress_fraction(), None);

    // Determinate caching phase (25 MB of 50 MB copied)
    if let Some(ref mut ft) = state.file_transfer {
        ft.status_text = "Caching from Google Drive...".to_string();
        ft.bytes_copied = 25 * 1024 * 1024;
        ft.elapsed_secs = 3.0;
    }
    let snap2 = state.render_snapshot();
    let ft2 = snap2.file_transfer.as_ref().unwrap();
    let pct = ft2
        .progress_fraction()
        .expect("Expected determinate progress");
    assert!((pct - 0.5).abs() < 1e-5);
}

#[test]
fn test_open_stream_device_dialog_and_mic_input_mode_transitions() {
    let engine_src = include_str!("ui_overlay.rs");
    assert!(engine_src.contains("OPEN STREAM/DEVICE"));
    assert!(!engine_src.contains(concat!("OPEN URL", " / STREAM")));
    assert!(engine_src.contains("Open Stream / Device"));
    assert!(engine_src.contains("Audio Input Devices (Microphone / Line-In):"));

    let uri = crate::state::mic_uri_for_device("USB Condenser Mic");
    assert_eq!(uri, "mic://USB Condenser Mic");
    assert!(crate::state::is_mic_uri(&uri));
    assert_eq!(
        crate::state::device_from_mic_uri(&uri),
        Some("USB Condenser Mic")
    );
    assert!(!crate::state::is_mic_uri("https://example.com/stream.mp3"));

    let mut state = AppState::new("Test".to_string());
    assert!(!state.is_mic_input);
    assert!(!state.available_input_devices.is_empty());

    state.available_input_devices = vec![
        crate::audio::SYSTEM_DEFAULT_AUDIO_DEVICE.to_string(),
        "USB Condenser Mic".to_string(),
    ];
    state.set_mic_input_mode(true, Some("USB Condenser Mic".to_string()));
    assert!(state.is_mic_input);
    assert_eq!(
        state.selected_audio_device.as_deref(),
        Some("USB Condenser Mic")
    );
    assert_eq!(state.available_audio_devices, state.available_input_devices);

    state.file_loaded = true;
    state.playlist = std::sync::Arc::new(vec![uri.clone()]);
    state.song_title = "Microphone Input".to_string();
    assert!(
        !state.has_active_playback(),
        "Microphone capture mode must not be treated as active file playback for drop-to-queue"
    );

    let snap = state.render_snapshot();
    assert!(snap.is_mic_input);
    assert_eq!(snap.available_input_devices, state.available_input_devices);

    // Dropping a file while in microphone mode replaces the mic input and switches back to output mode
    state.track_info_rect = Some([0.0, 0.0, 400.0, 400.0]);
    state.handle_dropped_files(vec!["song.flac".to_string()], Some([100.0, 100.0]));
    assert!(!state.is_mic_input);
    assert_eq!(*state.playlist, vec!["song.flac".to_string()]);
    assert_eq!(state.load_request.as_deref(), Some("song.flac"));
}

#[test]
fn test_waveform_history_ring_slot_mapping() {
    const CAP: usize = WAVEFORM_HISTORY_RING_CAP;
    assert_eq!(CAP, 144);
    let mut ring = [0u64; CAP];
    let mut ring_head = 0usize;

    // Simulate 300 sequential pushes and verify both 144-row (2D) and 96-row (3D) windows
    // map logical [0..N] oldest->newest onto the exact physical ring slots.
    for seq in 1..=300u64 {
        ring[ring_head] = seq;
        ring_head = (ring_head + 1) % CAP;

        for logical_rows in [seq.min(144) as usize, seq.min(96) as usize] {
            let ring_base = compute_waveform_ring_base(ring_head, logical_rows) as usize;
            for hist_idx in 0..logical_rows {
                let phys_row = (ring_base + hist_idx) % CAP;
                let expected_seq = seq - (logical_rows - 1 - hist_idx) as u64;
                assert_eq!(
                    ring[phys_row], expected_seq,
                    "seq={seq}, logical_rows={logical_rows}, hist_idx={hist_idx}"
                );
            }
        }
    }

    // Steady-state paused frame with unchanged push_count and hist_len is NOT dirty
    assert!(!is_waveform_history_dirty(
        420, 420, 96, 96, 1024, 1024, 3, 3
    ));
    // Paused seek / clear (`waveform_history.clear()` sets hist_len = 0 without advancing push_count)
    // MUST mark history dirty so 3D/CRT oscilloscope rows resync/zero immediately instead of freezing
    assert!(is_waveform_history_dirty(420, 420, 0, 96, 1024, 1024, 3, 3));
    // Switching away from Visualizer 24 (`last_hist_len = usize::MAX`) forces resync even when hist_len == 0
    assert!(is_waveform_history_dirty(
        0,
        0,
        0,
        usize::MAX,
        1024,
        1024,
        3,
        24
    ));
}

#[test]
fn test_waveform_history_f16_pack_unpack_roundtrip_and_bandwidth_halving() {
    // Verify exact special cases (zero, signed zero, +/-1.0, subnormals, overflow -> Inf, NaN)
    assert_eq!(f32_to_f16_bits(0.0), 0x0000);
    assert_eq!(f32_to_f16_bits(-0.0), 0x8000);
    assert_eq!(f32_to_f16_bits(1.0), 0x3C00);
    assert_eq!(f32_to_f16_bits(-1.0), 0xBC00);
    assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
    assert_eq!(f16_bits_to_f32(0xBC00), -1.0);
    assert!(f16_bits_to_f32(f32_to_f16_bits(1.0e6)).is_infinite());
    assert!(f16_bits_to_f32(f32_to_f16_bits(f32::NAN)).is_nan());
    // Subnormal f16 roundtrip (~1e-6)
    let sub = 1.0e-6f32;
    let sub_rt = f16_bits_to_f32(f32_to_f16_bits(sub));
    assert!((sub_rt - sub).abs() < 1.0e-7, "sub_rt={sub_rt}");

    // Exhaustively verify the exact branchless WGSL `unpack2x16float_compat` bit-math across
    // ALL 65,536 possible IEEE-754 binary16 bit patterns (normals, subnormals, +/-0, Inf/NaN).
    let unpack2x16_wgsl_mirror = |packed_word: u32| -> [f32; 2] {
        let h = [packed_word & 0xFFFF, packed_word >> 16];
        let mut out = [0.0f32; 2];
        for lane in 0..2 {
            let sign_bits = (h[lane] & 0x8000) << 16;
            let exp = (h[lane] >> 10) & 0x1F;
            let mant_shifted = (h[lane] & 0x03FF) << 13;
            let norm_val =
                f32::from_bits(sign_bits | ((exp.wrapping_add(112)) << 23) | mant_shifted);
            let sub_mag = f32::from_bits(0x3880_0000 | mant_shifted) - 6.103_515_6e-5_f32;
            let sub_val = f32::from_bits(sub_mag.to_bits() | sign_bits);
            let finite_val = if exp == 0 { sub_val } else { norm_val };
            out[lane] = if exp == 31 { 0.0 } else { finite_val };
        }
        out
    };
    for pair_idx in 0..32768u32 {
        let lo = (pair_idx * 2) as u16;
        let hi = (pair_idx * 2 + 1) as u16;
        let word = (lo as u32) | ((hi as u32) << 16);
        let got = unpack2x16_wgsl_mirror(word);
        for (lane, bits) in [(0usize, lo), (1usize, hi)] {
            let exp = (bits >> 10) & 0x1F;
            if exp == 31 {
                assert_eq!(
                    got[lane], 0.0,
                    "Inf/NaN f16 bits {bits:#06x} must sanitize to 0.0"
                );
            } else {
                let expected = f16_bits_to_f32(bits);
                assert_eq!(
                    got[lane].to_bits(),
                    expected.to_bits(),
                    "Exact bit mismatch for f16 {bits:#06x}: got {} ({:#010x}), expected {} ({:#010x})",
                    got[lane],
                    got[lane].to_bits(),
                    expected,
                    expected.to_bits()
                );
            }
        }
    }

    // Pack a full 2048-sample normalized waveform row into 1024 u32 words (4 KB vs 8 KB)
    let mut src_row = [0.0f32; 2048];
    for (i, sample) in src_row.iter_mut().enumerate() {
        let phase = (i as f32 / 2048.0) * std::f32::consts::TAU * 7.0;
        *sample = phase.sin() * 0.95 + (phase * 3.0).cos() * 0.05;
    }
    let mut packed_row = [0u32; 1024];
    pack_wave_row_f16(&src_row, &mut packed_row);
    assert_eq!(std::mem::size_of_val(&packed_row), 4096);
    assert_eq!(std::mem::size_of_val(&src_row), 8192);

    // Verify WGSL unpack2x16float lane semantics: low 16 bits = even index (.x), high 16 bits = odd index (.y)
    for (flat_idx, &orig) in src_row.iter().enumerate() {
        let word = packed_row[flat_idx >> 1];
        let half_bits = if (flat_idx & 1) == 0 {
            (word & 0xFFFF) as u16
        } else {
            (word >> 16) as u16
        };
        let decoded = f16_bits_to_f32(half_bits);
        assert!(
            (decoded - orig).abs() < 6.0e-4,
            "idx={flat_idx}: orig={orig}, decoded={decoded}"
        );
    }
}

#[test]
fn test_negotiate_optional_gpu_features_graceful_fallback_and_full_enable() {
    // 1. Baseline / mobile / software adapter with zero optional features: must succeed with empty set
    let baseline = negotiate_optional_gpu_features(wgpu::Features::empty(), true, true);
    assert!(baseline.required_features.is_empty());
    assert!(!baseline.shader_f16);
    assert!(!baseline.subgroup);
    assert!(!baseline.subgroup_barrier);
    assert!(!baseline.indirect_first_instance);
    assert!(!baseline.multi_draw_indirect_count);
    assert!(!baseline.texture_format_16bit_norm);
    assert!(!baseline.pipeline_cache);
    assert!(!baseline.timestamp_query);

    // 2. Modern desktop adapter (e.g. RDNA 4 gfx1201) advertising full feature set
    let modern_caps = wgpu::Features::SHADER_F16
        | wgpu::Features::SUBGROUP
        | wgpu::Features::SUBGROUP_BARRIER
        | wgpu::Features::INDIRECT_FIRST_INSTANCE
        | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT
        | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::PIPELINE_CACHE
        | wgpu::Features::TIMESTAMP_QUERY;
    let negotiated = negotiate_optional_gpu_features(modern_caps, false, true);
    assert!(negotiated.shader_f16);
    assert!(negotiated.subgroup);
    assert!(negotiated.subgroup_barrier);
    assert!(negotiated.indirect_first_instance);
    assert!(negotiated.multi_draw_indirect_count);
    assert!(negotiated.texture_format_16bit_norm);
    assert!(negotiated.pipeline_cache);
    assert!(
        !negotiated.timestamp_query,
        "Timestamps must remain opt-in when enable_timestamps is false"
    );
}

#[test]
fn test_host_f64_modular_time_precision_after_48_hours() {
    // Simulate 48 hours of monotonic f64 playback at 144 FPS wrapped into [P, 2*P)
    let dt = 1.0f64 / 144.0;
    let play_time = 48.0 * 3600.0 * 2.88_f64;
    let t0 = wrap_host_phase_time(play_time);
    let play_time_next = play_time + dt * 2.88;
    let t1 = wrap_host_phase_time(play_time_next);

    assert!(
        (t0 as f64) >= HOST_TIME_WRAP_PERIOD_SECS && (t0 as f64) < 2.0 * HOST_TIME_WRAP_PERIOD_SECS,
        "t0={t0} must lie in [P, 2*P)"
    );
    // If wrap boundary was not crossed, f32 delta must match 2.88/144 = 0.02s within < 1ms!
    if t1 > t0 {
        let delta = t1 - t0;
        assert!(
            (delta - 0.02).abs() < 0.001,
            "Expected smooth 20ms anim step after 48h, got {delta}"
        );
    }

    // Verify smooth_time > 0.0 at EVERY wrap multiple k*P (k >= 1) so `select(audio.time, audio.smooth_time, audio.smooth_time > 0.0)` never glitches!
    for k in 1..=8 {
        let exact_wrap = wrap_host_phase_time((k as f64) * HOST_TIME_WRAP_PERIOD_SECS);
        assert!(
            exact_wrap > 0.0,
            "wrap_host_phase_time({k}*P) must stay strictly > 0.0, got {exact_wrap}"
        );
    }

    // Verify exact 2π phase continuity across the [P, 2*P) -> P wrap boundary for BOTH integer
    // AND decimal shader frequencies (0.18, 0.225, 0.35, 0.45, 0.86, 0.92, 1.2, 3.2)!
    let pre_wrap = wrap_host_phase_time(2.0 * HOST_TIME_WRAP_PERIOD_SECS - 1e-4);
    let post_wrap = wrap_host_phase_time(2.0 * HOST_TIME_WRAP_PERIOD_SECS + 1e-4);
    for freq in [1.0f32, 0.18, 0.225, 0.35, 0.45, 0.86, 0.92, 1.2, 3.2] {
        let diff_sin = ((pre_wrap * freq).sin() - (post_wrap * freq).sin()).abs();
        let diff_cos = ((pre_wrap * freq).cos() - (post_wrap * freq).cos()).abs();
        assert!(
            diff_sin < 5e-3 && diff_cos < 5e-3,
            "Decimal shader frequency {freq} jumped across wrap: pre={pre_wrap}, post={post_wrap}, diff_sin={diff_sin}, diff_cos={diff_cos}"
        );
    }

    assert_eq!(wrap_host_phase_time(0.0), 0.0);
    assert_eq!(wrap_host_phase_time(-1.0), 0.0);
    assert_eq!(wrap_host_phase_time(f64::NAN), 0.0);
    assert_eq!(wrap_host_phase_time(f64::INFINITY), 0.0);
}

#[test]
fn test_phase3_video_two_plane_border_blur_and_indirect_draws() {
    // 1. Validate vis_video.wgsl compiles and exposes both fs_border_blur and fs_main entry points
    let video_wgsl = include_str!("../../../../src/shaders/vis_video.wgsl");
    let module =
        wgpu::naga::front::wgsl::parse_str(video_wgsl).expect("vis_video.wgsl must parse cleanly");
    let mut validator = wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    );
    validator
        .validate(&module)
        .expect("vis_video.wgsl must validate under Capabilities::empty()");

    let ep_names: Vec<&str> = module
        .entry_points
        .iter()
        .map(|ep| ep.name.as_str())
        .collect();
    assert!(ep_names.contains(&"vs_main"));
    assert!(ep_names.contains(&"fs_border_blur"));
    assert!(ep_names.contains(&"fs_main"));
    assert!(
        video_wgsl.contains("params.chroma_layout == 1u")
            && video_wgsl.contains("params.chroma_layout == 2u"),
        "vis_video.wgsl must support 2-plane SemiPlanar420 (NV12/P010) and SemiPlanarVu420 (NV21)"
    );
    assert!(
        !video_wgsl.contains("let half_s = 15"),
        "vis_video.wgsl fs_main must not run a 93-fetch per-pixel border loop"
    );

    let common_wgsl = include_str!("../../../../src/shaders/_common.wgsl");
    assert!(
        common_wgsl.contains("fn unpack2x16float_compat(")
            && !common_wgsl.contains("decode_f16_bits"),
        "_common.wgsl must use branchless dual-lane unpack2x16float_compat under Capabilities::empty()"
    );

    // 2. Verify Visualizer 23 uses draw_indexed_indirect (no unconditional 0..65536 splash draw)
    let prod_code = std::fs::read_to_string(workspace_path("crates/rt-render/src/engine/mod.rs"))
        .expect("engine/mod.rs must exist");
    assert!(
        !prod_code.contains("draw_indexed(0..6, 0, 0..65536)"),
        "Visualizer 23 must not unconditionally draw 65,536 splash instances"
    );
    assert!(
        prod_code.contains("draw_indexed_indirect(&self.lyrics_indirect_args_buffer, 0)")
            && prod_code.contains("draw_indexed_indirect(&self.splash_indirect_args_buffer, 0)"),
        "Visualizer 23 must use draw_indexed_indirect for both lyrics and compacted droplets"
    );
    assert!(
        !prod_code.contains("draw(0..6,"),
        "Fullscreen passes must use 3-vertex oversized fullscreen triangles, never 6-vertex quads"
    );
}

#[test]
fn test_acoustic_wave_chamber_registration_and_phase_extraction() {
    assert!(
        crate::state::VISUALIZERS
            .iter()
            .all(|v| v.id != 22 && v.id != 30),
        "Legacy Neon Spatial Room (ID 22) and Acoustic Wave Chamber (ID 30) must be removed"
    );

    let studio = crate::state::VISUALIZERS
        .iter()
        .find(|v| v.id == 31)
        .expect("Visualizer ID 31 (Neon Acoustic Studio) must be registered");
    assert_eq!(studio.name, "Neon Acoustic Studio");
    assert_eq!(studio.filename, "vis_neon_acoustic_room.wgsl");
    assert!(studio.requires_acoustic_waves);
    assert!(!studio.heavy_shader);
    assert!(matches!(
        studio.pipeline_type,
        crate::state::PipelineType::Mesh3D {
            geometry: crate::state::Geometry::NeonRoom,
            instances: 1,
        }
    ));
    let mut st = crate::state::AppState::new("smoke_default_check.flac".to_string());
    assert!(
        st.room_volume_smoke,
        "Neon Acoustic Studio smoke must default ON"
    );
    st.room_volume_smoke = false;
    assert!(
        !st.render_snapshot().room_volume_smoke,
        "render_snapshot() must preserve room_volume_smoke"
    );

    let sine_wave: Vec<f32> = (0..256).map(|k| (k as f32 * 0.25).sin()).collect();
    let inv_wave: Vec<f32> = sine_wave.iter().map(|&s| -s).collect();
    let mut spectrum = vec![0.0f32; 1024];
    spectrum[1] = 50.0;
    spectrum[12] = 40.0;
    spectrum[40] = 30.0;
    let mut spatial = [0.0f32; 16];
    spatial[0] = 0.9;
    spatial[1] = 0.9;

    let mut phases = [0.0f32; 32];
    let in_phase_channels = vec![sine_wave.clone(), sine_wave.clone()];
    for f in 0..16 {
        VulkanEngine::extract_acoustic_wave_phases(
            &mut phases,
            &in_phase_channels,
            &spectrum,
            &spatial,
            2,
            1.0 / 60.0,
            true,
            f as f32 * (1.0 / 60.0),
        );
    }
    assert!(
        phases[0].abs() < 1e-4 && phases[1].abs() < 0.08,
        "In-phase L==R must yield zero inter-channel phase delta, got {:?}",
        &phases[0..2]
    );

    let out_phase_channels = vec![sine_wave, inv_wave];
    for f in 0..24 {
        VulkanEngine::extract_acoustic_wave_phases(
            &mut phases,
            &out_phase_channels,
            &spectrum,
            &spatial,
            2,
            1.0 / 60.0,
            true,
            f as f32 * (1.0 / 60.0),
        );
    }
    assert!(
        (phases[1].abs() - std::f32::consts::PI).abs() < 0.08,
        "Inverted polarity L==-R must converge to +-PI phase delta, got {}",
        phases[1]
    );

    for step in 0..500 {
        VulkanEngine::extract_acoustic_wave_phases(
            &mut phases,
            &out_phase_channels,
            &spectrum,
            &spatial,
            2,
            1.0 / 60.0,
            true,
            step as f32 * 0.05,
        );
        assert!(
            phases[17] >= 0.0 && phases[17] < std::f32::consts::TAU,
            "base_osc must wrap continuously inside [0, TAU)"
        );
        for obj_idx in 0..3 {
            let x = phases[18 + obj_idx * 4];
            let y = phases[19 + obj_idx * 4];
            let z = phases[20 + obj_idx * 4];
            assert!(x.abs() < 10.8 && y > -1.5 && y < 4.9 && z.abs() < 8.9);
        }
        assert!(phases[31] >= 0.0 && phases[31] <= 1.0);
    }
}

#[test]
fn test_neon_acoustic_studio_camera_choreography_and_clearance() {
    let obstacles: &[(&str, [f32; 3], [f32; 3])] = &[
        ("FL tower", [-3.8, -0.25, 5.5], [0.51, 1.15, 0.54]),
        ("FR tower", [3.8, -0.25, 5.5], [0.51, 1.15, 0.54]),
        ("Center cabinet", [0.0, -0.02, 5.32], [0.93, 0.35, 0.35]),
        ("LFE sub", [0.0, -0.90, 5.50], [1.28, 0.51, 0.58]),
        ("SL monitor", [-6.2, -0.05, 0.5], [0.38, 1.18, 0.38]),
        ("SR monitor", [6.2, -0.05, 0.5], [0.38, 1.18, 0.38]),
        ("RL tower", [-4.2, 0.1, -4.8], [0.51, 1.15, 0.54]),
        ("RR tower", [4.2, 0.1, -4.8], [0.51, 1.15, 0.54]),
        ("TFL Atmos", [-3.2, 4.32, 3.5], [0.38, 0.28, 0.38]),
        ("TFR Atmos", [3.2, 4.32, 3.5], [0.38, 0.28, 0.38]),
        ("TRL Atmos", [-3.2, 4.32, -2.5], [0.38, 0.28, 0.38]),
        ("TRR Atmos", [3.2, 4.32, -2.5], [0.38, 0.28, 0.38]),
    ];

    const STEPS: usize = 1080;
    const DT_REAL: f64 = 0.05;
    let mut samples = Vec::with_capacity(STEPS);

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;
    let mut min_dir_x = f32::MAX;
    let mut min_dir_z = f32::MAX;
    let mut max_dir_z = f32::MIN;

    for idx in 0..STEPS {
        let real_t = idx as f64 * DT_REAL;
        let (eye, target) = compute_neon_acoustic_studio_camera(real_t * 2.88);
        assert!(
            eye.is_finite() && target.is_finite(),
            "Non-finite camera coordinates at t={real_t:.2}s"
        );
        assert!(
            eye.x.abs() < 9.5 && eye.y > -0.8 && eye.y < 3.85 && eye.z > -8.2 && eye.z < 7.5,
            "Camera out of safe room interior bounds at t={real_t:.2}s: {eye:?}"
        );
        if eye.z > 3.5 && eye.x.abs() < 5.4 {
            assert!(
                eye.y > -0.7,
                "Camera clipped stage riser at t={real_t:.2}s: {eye:?}"
            );
        }
        for &(label, center, half) in obstacles {
            let in_x = (eye.x - center[0]).abs() < (half[0] + 0.50);
            let in_y = (eye.y - center[1]).abs() < (half[1] + 0.50);
            let in_z = (eye.z - center[2]).abs() < (half[2] + 0.50);
            assert!(
                !(in_x && in_y && in_z),
                "Camera violated 0.5m clearance of {label} at t={real_t:.2}s: {eye:?}"
            );
        }

        let gaze = target - eye;
        let gaze_len = gaze.length();
        assert!(
            gaze_len > 3.5,
            "Degenerate short look_at vector ({gaze_len:.2}m) at t={real_t:.2}s"
        );
        let dir = gaze / gaze_len;
        assert!(
            dir.y.abs() < 0.85,
            "Steep look_at pitch (dir.y={:.3}) at t={real_t:.2}s",
            dir.y
        );

        min_x = min_x.min(eye.x);
        max_x = max_x.max(eye.x);
        min_y = min_y.min(eye.y);
        max_y = max_y.max(eye.y);
        min_z = min_z.min(eye.z);
        max_z = max_z.max(eye.z);
        min_dir_x = min_dir_x.min(dir.x);
        min_dir_z = min_dir_z.min(dir.z);
        max_dir_z = max_dir_z.max(dir.z);

        samples.push((real_t, eye, dir));
    }

    let mut dwell_speed_sum = 0.0f32;
    let mut dwell_count = 0usize;
    let mut peak_trans_speed = 0.0f32;
    for idx in 0..STEPS {
        let (real_t, eye_curr, dir_curr) = samples[idx];
        let (_, eye_next, dir_next) = samples[(idx + 1) % STEPS];
        let step_dist = (eye_next - eye_curr).length();
        let dir_delta = (dir_next - dir_curr).length();
        assert!(
            step_dist < 0.40 && dir_delta < 0.12,
            "Camera jump at t={real_t:.2}s: step_dist={step_dist:.4}, dir_delta={dir_delta:.4}"
        );

        let speed = step_dist / (DT_REAL as f32);
        let local_u = ((real_t % NEON_STUDIO_CAM_SHOT_SECS) / NEON_STUDIO_CAM_SHOT_SECS) as f32;
        if local_u < 0.70 {
            dwell_speed_sum += speed;
            dwell_count += 1;
        } else if local_u > 0.74 && local_u < 0.98 {
            peak_trans_speed = peak_trans_speed.max(speed);
        }
    }

    let avg_dwell_speed = dwell_speed_sum / (dwell_count as f32);
    assert!(
        avg_dwell_speed < 0.85 && peak_trans_speed > 2.5,
        "Strategic Dwell & Glide contrast invalid: avg_dwell={avg_dwell_speed:.3} m/s, peak_trans={peak_trans_speed:.3} m/s"
    );
    assert!(
        min_x < -7.0
            && max_x > 7.0
            && min_z < -7.0
            && max_z > 6.5
            && min_y < 0.05
            && max_y > 3.4
            && max_dir_z > 0.7
            && min_dir_z < -0.7
            && min_dir_x < -0.8,
        "Incomplete room coverage: x=[{min_x:.2},{max_x:.2}], y=[{min_y:.2},{max_y:.2}], z=[{min_z:.2},{max_z:.2}], dir_x_min={min_dir_x:.2}, dir_z=[{min_dir_z:.2},{max_dir_z:.2}]"
    );

    let (hi_eye, hi_target) = compute_neon_acoustic_studio_camera(2.0 * HOST_TIME_WRAP_PERIOD_SECS);
    assert!(
        hi_eye.is_finite()
            && hi_target.is_finite()
            && (hi_target - hi_eye).length() > 3.5
            && hi_eye.x.abs() < 9.5
    );

    let (mesh_verts, _mesh_inds) = generate_neon_room_mesh();
    let mut min_y_c2 = f32::MAX;
    let mut sum_x_c2 = 0.0f32;
    let mut sum_z_c2 = 0.0f32;
    let mut cnt_c2 = 0usize;
    let mut max_y_c3 = f32::MIN;
    let mut sum_x_c3 = 0.0f32;
    let mut sum_z_c3 = 0.0f32;
    let mut cnt_c3 = 0usize;
    for v in &mesh_verts {
        if (v.tex_coords[0] - 2.0).abs() < 0.1 {
            min_y_c2 = min_y_c2.min(v.position[1]);
            sum_x_c2 += v.position[0];
            sum_z_c2 += v.position[2];
            cnt_c2 += 1;
        } else if (v.tex_coords[0] - 3.0).abs() < 0.1 && v.tex_coords[1] > 1.1 {
            max_y_c3 = max_y_c3.max(v.position[1]);
            sum_x_c3 += v.position[0];
            sum_z_c3 += v.position[2];
            cnt_c3 += 1;
        }
    }
    assert!(cnt_c2 > 0 && cnt_c3 > 0);
    let avg_x_c2 = sum_x_c2 / cnt_c2 as f32;
    let avg_z_c2 = sum_z_c2 / cnt_c2 as f32;
    let avg_x_c3 = sum_x_c3 / cnt_c3 as f32;
    let avg_z_c3 = sum_z_c3 / cnt_c3 as f32;
    assert!(
        min_y_c2 >= max_y_c3 + 0.05
            && (avg_x_c2 - avg_x_c3).abs() < 0.05
            && (avg_z_c2 - avg_z_c3).abs() < 0.30,
        "Center (ch 2) must be cleanly stacked above LFE Sub (ch 3): min_y_c2={min_y_c2:.3}, max_y_c3={max_y_c3:.3}, avg_c2=({avg_x_c2:.2},{avg_z_c2:.2}), avg_c3=({avg_x_c3:.2},{avg_z_c3:.2})"
    );
}

#[test]
fn test_biolum_refactor_mesh_camera_and_shaders() {
    let biolum_vis = crate::state::VISUALIZERS
        .iter()
        .find(|v| v.id == 20)
        .expect("Visualizer ID 20 (Bioluminescent Waves) must be registered in VISUALIZERS");
    assert_eq!(biolum_vis.name, "Bioluminescent Waves");
    assert_eq!(biolum_vis.filename, "vis_bioluminescence.wgsl");
    assert!(!biolum_vis.heavy_shader);
    assert_eq!(
        biolum_vis.pipeline_type,
        crate::state::PipelineType::Mesh3D {
            geometry: crate::state::Geometry::UnitQuad,
            instances: BIOLUM_PARTICLE_COUNT,
        }
    );
    assert_eq!(BIOLUM_PARTICLE_COUNT, 36_864);
    assert_eq!(BIOLUM_WORKGROUPS * 256, BIOLUM_PARTICLE_COUNT);

    let default_state = AppState::new("Test App".to_string());
    assert!(
        !default_state.biolum_top_down,
        "Bioluminescent Waves must default to 3D Perspective Horizon camera"
    );

    let (ocean_verts, ocean_inds) = generate_biolum_ocean_mesh();
    assert_eq!(ocean_verts.len(), 161 * 141);
    assert_eq!(ocean_inds.len(), 160 * 140 * 6);
    for &idx in &ocean_inds {
        assert!((idx as usize) < ocean_verts.len());
    }
    let mut min_u = f32::MAX;
    let mut max_u = f32::MIN;
    let mut min_v = f32::MAX;
    let mut max_v = f32::MIN;
    for v in &ocean_verts {
        min_u = min_u.min(v.tex_coords[0]);
        max_u = max_u.max(v.tex_coords[0]);
        min_v = min_v.min(v.tex_coords[1]);
        max_v = max_v.max(v.tex_coords[1]);
    }
    assert!((min_u - 0.0).abs() < 1e-6 && (max_u - 1.0).abs() < 1e-6);
    assert!((min_v - 0.0).abs() < 1e-6 && (max_v - 1.0).abs() < 1e-6);

    let (persp_eye, persp_target, persp_up) = compute_biolum_camera(10.0, false);
    assert!(persp_eye.y > 0.0 && (persp_target.z - persp_eye.z) > 0.0 && persp_up == glam::Vec3::Y);
    let (td_eye, td_target, td_up) = compute_biolum_camera(10.0, true);
    assert!(td_eye.y > 10.0 && td_target.y.abs() < 1e-5 && (td_up.z - 1.0).abs() < 1e-5);

    const SHADER_COMMON: &str = include_str!("../../../../src/shaders/_common.wgsl");
    let raw_biolum_src = include_str!("../../../../src/shaders/vis_bioluminescence.wgsl");
    let biolum_src = raw_biolum_src.replace("// INCLUDE: common", SHADER_COMMON);
    let biolum_compute_src = include_str!("../../../../src/shaders/biolum_compute.wgsl")
        .replace("// INCLUDE: common", SHADER_COMMON);

    for fn_name in [
        "fn vs_bg",
        "fn fs_bg",
        "fn vs_ocean",
        "fn fs_ocean",
        "fn vs_main_3d",
        "fn fs_main",
    ] {
        assert!(
            raw_biolum_src.contains(fn_name),
            "vis_bioluminescence.wgsl must define {fn_name}"
        );
    }
    for legacy_hack in ["bezel_mask", "scanline", "distorted_ndc"] {
        assert!(
            !raw_biolum_src.contains(legacy_hack),
            "vis_bioluminescence.wgsl must not contain legacy CRT/bezel hack {legacy_hack}"
        );
    }

    for (name, src) in [
        ("vis_bioluminescence.wgsl", &biolum_src),
        ("biolum_compute.wgsl", &biolum_compute_src),
    ] {
        let mut frontend = wgpu::naga::front::wgsl::Frontend::new();
        let module = frontend
            .parse(src)
            .unwrap_or_else(|e| panic!("{name} parse error: {}", e.emit_to_string(src)));
        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name} validation error: {e:?}"));
    }
}
