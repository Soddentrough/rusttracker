use super::{
    AppState, Args, KeyCode, Parser, advance_playlist_on_track_ended,
    apply_gamepad_stick_panel_resize, apply_ipc_file_paths_to_state,
    apply_picked_file_paths_to_state, clamp_panel_split_ratio, clamp_window_geometry_to_monitor,
    extract_rusttracker_shortcut_appids, handle_tui_key_event, handle_tui_mouse_event,
    provision_steam_shortcut_artwork_in_root, upsert_rusttracker_shortcut_in_vdf,
};
use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn collect_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("Failed to read dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn concat_dir_rs(dir: &std::path::Path, skip_tests_rs: bool) -> String {
    let mut files = Vec::new();
    collect_rs_files(dir, &mut files);
    let mut out = String::new();
    for p in files {
        if skip_tests_rs && p.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
            continue;
        }
        out.push_str(
            &std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", p.display())),
        );
        out.push('\n');
    }
    out
}

#[test]
fn test_window_geometry_fits_1080p_at_150_percent_scale() {
    // 1920x1080 at 150% DPI scaling -> 1280x720 logical monitor area
    let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
        clamp_window_geometry_to_monitor(1920, 1080, 0, 0, 1.5, 1440.0, 840.0);

    let est_outer_phys_w = ((w + 16.0) * 1.5).round() as i32;
    let est_outer_phys_h = ((h + 38.0) * 1.5).round() as i32;

    assert!(w < 1280.0 && h < 720.0);
    assert!(min_w <= w && min_h <= h);
    assert!(pos_x >= 0 && pos_y >= 0);
    assert!(pos_x + est_outer_phys_w <= 1920);
    assert!(pos_y + est_outer_phys_h <= 1080);
}

#[test]
fn test_window_geometry_fits_1080p_at_125_percent_scale() {
    // 1920x1080 at 125% DPI scaling -> 1536x864 logical monitor area
    let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
        clamp_window_geometry_to_monitor(1920, 1080, 0, 0, 1.25, 1440.0, 840.0);

    let est_outer_phys_w = ((w + 16.0) * 1.25).round() as i32;
    let est_outer_phys_h = ((h + 38.0) * 1.25).round() as i32;

    assert!(w <= 1440.0 && h < 840.0);
    assert!(min_w <= w && min_h <= h);
    assert!(pos_x >= 0 && pos_y >= 0);
    assert!(pos_x + est_outer_phys_w <= 1920);
    assert!(pos_y + est_outer_phys_h <= 1080);
}

#[test]
fn test_window_geometry_fits_1366x768_display() {
    let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
        clamp_window_geometry_to_monitor(1366, 768, 0, 0, 1.0, 1440.0, 840.0);

    let est_outer_phys_w = (w + 16.0).round() as i32;
    let est_outer_phys_h = (h + 38.0).round() as i32;

    assert!(w < 1366.0 && h < 768.0);
    assert!(min_w <= w && min_h <= h);
    assert!(pos_x >= 0 && pos_y >= 0);
    assert!(pos_x + est_outer_phys_w <= 1366);
    assert!(pos_y + est_outer_phys_h <= 768);
}

#[test]
fn test_window_geometry_preserves_default_on_large_display() {
    let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
        clamp_window_geometry_to_monitor(2560, 1440, -2560, 0, 1.0, 1440.0, 840.0);

    assert_eq!((w, h), (1440.0, 840.0));
    assert_eq!((min_w, min_h), (1024.0, 600.0));
    assert!(pos_x >= -2560 && pos_x + 1456 <= 0);
    assert!(pos_y >= 0 && pos_y + 878 <= 1440);
}

#[test]
fn test_window_geometry_preserves_full_steamdeck_1280x800_display() {
    let ((w, h), (min_w, min_h), (pos_x, pos_y)) =
        clamp_window_geometry_to_monitor(1280, 800, 0, 0, 1.0, 1280.0, 800.0);

    assert_eq!((w, h), (1280.0, 800.0));
    assert_eq!((min_w, min_h), (1024.0, 600.0));
    assert_eq!((pos_x, pos_y), (0, 0));
}

#[test]
fn test_gamepad_stick_resizes_panel_split_with_left_or_right_stick() {
    let mut state = AppState::new("test.mod".to_string());
    state.file_loaded = true;
    state.panel_split_ratio = 0.5;

    // Left stick UP (+1.0) shrinks top panel (moves split bar up)
    assert!(apply_gamepad_stick_panel_resize(&mut state, 1.0, 0.0, 0.1));
    assert!((state.panel_split_ratio - 0.4).abs() < 1e-5);

    // Right stick DOWN (-1.0) expands top panel (moves split bar down)
    assert!(apply_gamepad_stick_panel_resize(&mut state, 0.0, -1.0, 0.2));
    assert!((state.panel_split_ratio - 0.6).abs() < 1e-5);

    // Stick inside deadzone (<= 0.1) does not change ratio
    assert!(!apply_gamepad_stick_panel_resize(
        &mut state, 0.05, -0.09, 0.1
    ));
    assert!((state.panel_split_ratio - 0.6).abs() < 1e-5);

    // Moving stick while HUD is hidden restores HUD visibility
    state.show_hud = false;
    state.video_mode = 3;
    assert!(apply_gamepad_stick_panel_resize(&mut state, 0.5, 0.0, 0.1));
    assert!(state.show_hud);
    assert_eq!(state.video_mode, 0);

    // Modal dialogs suppress background panel resizing
    state.show_vis_picker = true;
    let before = state.panel_split_ratio;
    assert!(!apply_gamepad_stick_panel_resize(&mut state, 1.0, 1.0, 0.2));
    assert_eq!(state.panel_split_ratio, before);
}

#[test]
fn test_clamp_panel_split_ratio_allows_full_steamdeck_range() {
    // On 1280x800 Steam Deck, min_h = 120px corresponds to [0.15, 0.85]
    assert!((clamp_panel_split_ratio(0.10, 800.0) - 0.15).abs() < 1e-5);
    assert!((clamp_panel_split_ratio(0.90, 800.0) - 0.85).abs() < 1e-5);
    assert!((clamp_panel_split_ratio(0.20, 800.0) - 0.20).abs() < 1e-5);
    assert!((clamp_panel_split_ratio(0.80, 800.0) - 0.80).abs() < 1e-5);
}

#[test]
fn test_extract_and_provision_steam_shortcut_artwork() {
    // Construct a realistic binary shortcuts.vdf with 1 RustTracker entry (appid 0xEA304690 = 3929032336)
    // and 1 unrelated entry (appid 0x81234567)
    let mut vdf = Vec::new();
    vdf.extend_from_slice(b"\x00shortcuts\x00");
    vdf.extend_from_slice(b"\x000\x00");
    vdf.extend_from_slice(b"\x02appid\x00\x90\x46\x30\xea");
    vdf.extend_from_slice(b"\x01AppName\x00RustTracker-SteamDeck-GamePad.AppImage\x00");
    vdf.extend_from_slice(
        b"\x01Exe\x00\"/home/deck/Downloads/RustTracker-SteamDeck-GamePad.AppImage\"\x00",
    );
    vdf.extend_from_slice(b"\x01icon\x00\x00");
    vdf.extend_from_slice(b"\x00tags\x00\x08\x08");
    vdf.extend_from_slice(b"\x001\x00");
    vdf.extend_from_slice(b"\x02appid\x00\x67\x45\x23\x81");
    vdf.extend_from_slice(b"\x01AppName\x00OtherApp\x00");
    vdf.extend_from_slice(b"\x01Exe\x00\"/usr/bin/other\"\x00\x08");
    vdf.push(0x08);

    let appids = extract_rusttracker_shortcut_appids(&vdf);
    assert_eq!(appids, vec![3929032336]);

    let tmp_root =
        std::env::temp_dir().join(format!("rusttracker_steamgrid_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_root);
    let config_dir = tmp_root.join("userdata/8232340/config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("shortcuts.vdf"), &vdf).unwrap();

    let steamgrid_dir = std::path::Path::new("assets/steamgrid");
    let icon_png = std::path::Path::new("assets/icons/hicolor/512x512/apps/rusttracker.png");
    let count = provision_steam_shortcut_artwork_in_root(&tmp_root, steamgrid_dir, Some(icon_png));
    assert_eq!(count, 1);

    // Verify shortcuts.vdf had AppName normalized to "RustTracker" and icon populated
    let updated_vdf = std::fs::read(config_dir.join("shortcuts.vdf")).unwrap();
    assert!(
        updated_vdf
            .windows(b"\x01AppName\x00RustTracker\x00".len())
            .any(|w| w == b"\x01AppName\x00RustTracker\x00")
    );
    assert!(
        updated_vdf
            .windows(b"rusttracker.png\x00".len())
            .any(|w| w == b"rusttracker.png\x00")
    );

    assert!(config_dir.join("grid/3929032336.jpg").exists());
    assert!(config_dir.join("grid/3929032336p.jpg").exists());
    assert!(config_dir.join("grid/3929032336_hero.jpg").exists());
    assert!(config_dir.join("grid/3929032336_logo.png").exists());
    assert!(config_dir.join("grid/3929032336_icon.png").exists());
    assert!(
        tmp_root
            .join("appcache/librarycache/3929032336/header.jpg")
            .exists()
    );
    assert!(
        tmp_root
            .join("appcache/librarycache/3929032336_header.jpg")
            .exists()
    );
    assert!(
        tmp_root
            .join("appcache/librarycache/3929032336/library_600x900.jpg")
            .exists()
    );

    // Verify upsert into an empty shortcuts.vdf creates a valid RustTracker entry
    let (created_vdf, created_id) = upsert_rusttracker_shortcut_in_vdf(
        &[],
        "/home/deck/Downloads/RustTracker-SteamDeck.AppImage",
        "/home/deck/Downloads/",
        "/home/deck/.local/share/icons/hicolor/512x512/apps/rusttracker.png",
    );
    assert_eq!(created_id, 3929032336);
    assert_eq!(
        extract_rusttracker_shortcut_appids(&created_vdf),
        vec![3929032336]
    );
    let _ = std::fs::remove_dir_all(&tmp_root);
}

#[test]
fn test_cli_surround_upmix_flag_and_state_toggle() {
    let args_long = Args::try_parse_from(["rusttracker", "--surround-upmix"]).unwrap();
    assert!(args_long.surround_upmix);

    let args_alias = Args::try_parse_from(["rusttracker", "--upmix"]).unwrap();
    assert!(args_alias.surround_upmix);

    let args_default = Args::try_parse_from(["rusttracker"]).unwrap();
    assert!(!args_default.surround_upmix);

    assert!(crate::state::SurroundUpmixConfig::default().enabled);
    assert!(!crate::state::SurroundUpmixConfig::default().user_overridden);
    assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(2, 2));
    assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(6, 6));
    assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(8, 8));
    assert!(!crate::state::SurroundUpmixConfig::should_enable_by_default(12, 12));
    assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
        2, 6
    ));
    assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
        2, 8
    ));
    assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
        2, 12
    ));
    assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
        6, 8
    ));
    assert!(crate::state::SurroundUpmixConfig::should_enable_by_default(
        6, 12
    ));

    let mut state = AppState::new("Test Track".to_string());
    state.surround_upmix.enabled =
        crate::state::SurroundUpmixConfig::should_enable_by_default(2, 2);
    state.force_stereo_downmix = true;
    state.show_hud = false;
    state.video_mode = 3;
    state.file_loaded = true;

    // 1. On a standard stereo (2ch) device, enabling surround upmix must NOT change num_channels
    state.hardware_channels = 2;
    state.num_channels = 2;
    state.toggle_surround_upmix();
    let is_active_2 = state.surround_upmix.is_active_for_hardware(2);
    assert!(state.surround_upmix.enabled);
    assert!(!is_active_2);
    assert!(!state.force_stereo_downmix);
    assert_eq!(
        state.num_channels, 2,
        "Stereo (2ch) output device must remain 2ch when surround upmix is enabled"
    );

    state.toggle_surround_upmix();
    assert!(!state.surround_upmix.enabled);
    assert_eq!(state.num_channels, 2);

    // 2. On a Bed-Only 5.1 (6ch) device, enabling surround upmix sets num_channels to 6 (not 12)
    state.hardware_channels = 6;
    state.show_hud = false;
    state.video_mode = 3;
    state.toggle_surround_upmix();
    let is_active_6 = state.surround_upmix.is_active_for_hardware(6);
    let has_height_6 = crate::state::SurroundUpmixConfig::has_height_channels(6);
    assert!(state.surround_upmix.enabled);
    assert!(is_active_6);
    assert!(!has_height_6);
    assert_eq!(state.num_channels, 6);
    assert!(state.show_hud);
    assert_eq!(state.video_mode, 0);

    state.toggle_surround_upmix();
    assert!(!state.surround_upmix.enabled);
    assert_eq!(state.num_channels, 2);

    // 3. On a Bed + Height 7.1.4 (12ch) device, enabling surround upmix while bitstream passthrough
    // is active exits bitstream mode and sets num_channels to 12
    state.hardware_channels = 12;
    state.passthrough_enabled = true;
    state.stats.bitstream_active = true;
    state.raw_channel_vus = vec![0.1; 6];
    state.set_surround_upmix(true);
    let has_height_12 = crate::state::SurroundUpmixConfig::has_height_channels(12);
    assert!(state.surround_upmix.enabled);
    assert!(has_height_12);
    assert!(!state.passthrough_enabled);
    assert_eq!(state.passthrough_change_request, Some(false));
    assert_eq!(state.num_channels, 12);

    // Verify disabling surround upmix restores intrinsic channel count from raw_channel_vus
    state.set_surround_upmix(false);
    assert!(!state.surround_upmix.enabled);
    assert_eq!(state.num_channels, 6);
}

#[test]
fn test_cumulative_seek_osd_and_reset() {
    let mut state = AppState::new("Test Track".to_string());

    // Single jump forward shows +5.0s
    state.record_seek_osd(5.0);
    assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));
    assert!((state.osd_timer - 2.0).abs() < 1e-5);

    // Repeated jump forward before 2.0s timer expires accumulates
    state.tick_osd_timer(0.5);
    state.record_seek_osd(5.0);
    assert_eq!(state.osd_text.as_deref(), Some("+10.0s"));
    assert!((state.osd_timer - 2.0).abs() < 1e-5);

    state.tick_osd_timer(0.8);
    state.record_seek_osd(5.0);
    assert_eq!(state.osd_text.as_deref(), Some("+15.0s"));
    assert!((state.osd_timer - 2.0).abs() < 1e-5);

    // After 2.0 seconds, the OSD expires and resets cumulative time
    state.tick_osd_timer(2.0);
    assert_eq!(state.osd_text, None);
    assert_eq!(state.osd_timer, 0.0);
    assert_eq!(state.cumulative_scrub, 0.0);

    // Next jump starts fresh at +5.0s
    state.record_seek_osd(5.0);
    assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));

    // Jumping in the opposite direction resets to the new direction and accumulates backward
    state.record_seek_osd(-5.0);
    assert_eq!(state.osd_text.as_deref(), Some("-5.0s"));
    state.record_seek_osd(-5.0);
    assert_eq!(state.osd_text.as_deref(), Some("-10.0s"));

    // Non-seek OSD message interrupts accumulation so subsequent seek starts fresh
    state.osd_text = Some("Paused".to_string());
    state.osd_timer = 2.0;
    state.record_seek_osd(5.0);
    assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));

    // Frame-stepping resets cumulative seconds
    state.record_seek_osd(1.0 / 60.0);
    assert_eq!(state.osd_text.as_deref(), Some("+1 Frame"));
    state.record_seek_osd(5.0);
    assert_eq!(state.osd_text.as_deref(), Some("+5.0s"));
}

#[test]
fn test_cli_mpegh_flag_and_state_toggle() {
    let args_mpegh = Args::try_parse_from(["rusttracker", "--mpegh"]).unwrap();
    assert!(args_mpegh.mpegh);
    assert!(!args_mpegh.no_mpegh);

    let args_mpegh_714 = Args::try_parse_from(["rusttracker", "--mpegh-714"]).unwrap();
    assert!(args_mpegh_714.mpegh);

    let args_no_mpegh = Args::try_parse_from(["rusttracker", "--no-mpegh"]).unwrap();
    assert!(args_no_mpegh.no_mpegh);
    assert!(!args_no_mpegh.mpegh);

    let default_cfg = crate::state::SurroundUpmixConfig::default();
    assert!(default_cfg.mpegh_bitstream);
    assert!(!default_cfg.mpegh_user_overridden);

    let mut state = AppState::new("Test MPEG-H".to_string());
    state.file_loaded = true;
    state.set_mpegh_bitstream(false);
    assert!(!state.surround_upmix.mpegh_bitstream);
    assert!(state.surround_upmix.mpegh_user_overridden);
    assert_eq!(state.passthrough_change_request, Some(false));

    state.passthrough_change_request = None;
    state.toggle_mpegh_bitstream();
    assert!(state.surround_upmix.mpegh_bitstream);
    assert!(state.surround_upmix.mpegh_user_overridden);
    assert!(state.surround_upmix.enabled);
    assert_eq!(state.passthrough_change_request, Some(false));
}

#[test]
fn test_middle_panel_defaults_to_pattern_view_and_flips_to_eq() {
    let mut state = AppState::new("Test Tracker.mod".to_string());
    state.file_loaded = true;
    state.source_channels = 2;
    state.hardware_channels = 8;

    // Surround upmix is enabled and active by default on surround hardware,
    // while the middle panel defaults to the Tracker Pattern / Heatmap viewer (`show_surround_eq == false`).
    assert!(state.surround_upmix.enabled);
    assert!(state.surround_upmix.is_active_for(2, 8));
    assert!(!state.show_surround_eq);
    assert!(!state.render_snapshot().show_surround_eq);

    // Flip middle panel to Surround EQ
    state.toggle_show_surround_eq();
    assert!(state.show_surround_eq);
    assert!(state.render_snapshot().show_surround_eq);

    // Flip back to Tracker Pattern / Heatmap viewer
    state.toggle_show_surround_eq();
    assert!(!state.show_surround_eq);

    // Disabling surround upmix while viewing EQ returns to Pattern View
    state.set_show_surround_eq(true);
    assert!(state.show_surround_eq);
    state.set_surround_upmix(false);
    assert!(!state.show_surround_eq);

    // Verify middle panel title bar buttons and consolidated upmix/layout labels in crates/rt-render/src/engine.rs
    let engine_src = concat_dir_rs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/rt-render/src/engine"),
        true,
    );
    assert!(engine_src.contains("SURROUND EQ"));
    assert!(engine_src.contains("PATTERN HEATMAP"));
    assert!(engine_src.contains("render_heatmap_eq_header"));
    assert!(!engine_src.contains("🎚 Surround EQ"));
    assert!(!engine_src.contains("📊 Pattern View"));
    assert!(!engine_src.contains("🎹 Piano Roll"));
    assert!(!engine_src.contains("🔊 SURROUND UPMIX: ON"));
    assert!(!engine_src.contains("11.1 (7.1.4): ON"));
    assert_eq!(crate::state::SurroundUpmixConfig::short_tag(8), "7.1");
    assert!(!crate::state::SurroundUpmixConfig::has_height_channels(8));
}

#[test]
fn test_tui_key_event_controls_and_multitrack_solo() {
    let mut state = AppState::new("MultiTrack.mkv".to_string());
    state.file_loaded = true;
    state.duration_seconds = 100.0;
    state.current_seconds = 20.0;
    state.audio_tracks = (0..3)
        .map(|id| crate::state::AudioTrackInfo {
            id,
            title: format!("Track {}", id + 1),
            codec: "FLAC".to_string(),
            channels: 2,
            sample_rate: 48000,
            language: None,
            is_atmos: false,
        })
        .collect();
    state.active_audio_tracks = vec![0, 1, 2];
    state.selected_audio_track = 0;

    // 1. Pressing '1' solos Track 1 from the initial full mix
    let should_quit = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
    );
    assert!(!should_quit);
    assert_eq!(state.selected_audio_track, 0);
    assert_eq!(state.active_audio_tracks, vec![0]);
    assert_eq!(state.audio_track_request, Some(0));

    // 2. Pressing '4' (num_tracks + 1) restores the full multi-track mix
    state.audio_mix_request = None;
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
    );
    assert_eq!(state.active_audio_tracks, vec![0, 1, 2]);
    assert!(state.audio_mix_request.is_some());

    // 3. View cycling with 'v' and Tab
    assert_eq!(
        state.effective_tui_center_view(),
        crate::state::TuiCenterView::Heatmap
    );
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
    );
    assert_eq!(
        state.effective_tui_center_view(),
        crate::state::TuiCenterView::SurroundEq
    );
    let _ = handle_tui_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        state.effective_tui_center_view(),
        crate::state::TuiCenterView::Heatmap
    );

    // 4. MPEG-H bitstream toggle ('g') & Mute/Unmute ('m') do not collide in SurroundEq;
    // Mute preserves pre-mute per-track volumes across unmute, and Bitstream passthrough ('b') resets SurroundEq/upmix
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
    );
    assert_eq!(
        state.effective_tui_center_view(),
        crate::state::TuiCenterView::SurroundEq
    );
    state.audio_track_volumes = vec![0.4, 0.8, 0.25];
    let initial_mpegh = state.surround_upmix.mpegh_bitstream;
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    assert_eq!(state.surround_upmix.mpegh_bitstream, !initial_mpegh);
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
    );
    assert_eq!(state.audio_track_volumes, vec![0.0, 0.0, 0.0]);
    assert_eq!(state.osd_text.as_deref(), Some("Muted"));
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
    );
    assert_eq!(state.audio_track_volumes, vec![0.4, 0.8, 0.25]);
    assert_eq!(state.osd_text.as_deref(), Some("Unmuted"));
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    assert_eq!(state.surround_upmix.mpegh_bitstream, initial_mpegh);

    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
    );
    if cfg!(any(target_os = "windows", target_os = "linux")) {
        assert!(state.passthrough_enabled);
        assert!(!state.surround_upmix.enabled);
        assert_eq!(
            state.effective_tui_center_view(),
            crate::state::TuiCenterView::Heatmap
        );
        assert_eq!(state.passthrough_change_request, Some(true));
    } else {
        assert!(!state.passthrough_enabled);
        assert!(state.passthrough_change_request.is_none());
    }

    // 5. Split ratio adjustment with '[' and ']'
    let initial_ratio = state.panel_split_ratio;
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
    );
    assert!(state.panel_split_ratio > initial_ratio);
    let _ = handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
    );
    assert!((state.panel_split_ratio - initial_ratio).abs() < 1e-4);

    // 6. Quit with Esc, 'q', and Ctrl+C
    assert!(handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ));
    assert!(handle_tui_key_event(
        &mut state,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ));
}

#[test]
fn test_tui_mouse_seek_and_center_panel_click() {
    let mut state = AppState::new("MouseTest.flac".to_string());
    state.file_loaded = true;
    state.duration_seconds = 200.0;
    let term_area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let meta_len = crate::ui::build_track_info_lines(&state, 30).len();
    let layout = crate::ui::compute_tui_layout(term_area, &state, meta_len);

    // Click at the midpoint of the timeline bar seeks to ~50% (100s)
    let inner_x = layout.timeline_rect.x + layout.timeline_rect.width / 2;
    let inner_y = layout.timeline_rect.y;
    let click_seek = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: inner_x,
        row: inner_y,
        modifiers: KeyModifiers::NONE,
    };
    handle_tui_mouse_event(&mut state, click_seek, term_area);
    let seek_target = state
        .seek_request
        .expect("Expected seek_request from timeline click");
    assert!(
        (seek_target - 100.0).abs() < 5.0,
        "Expected seek near 100.0s, got {seek_target}"
    );

    // Click inside the top-center panel cycles the center view
    let center_click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: layout.center_rect.x + 2,
        row: layout.center_rect.y + 2,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(
        state.effective_tui_center_view(),
        crate::state::TuiCenterView::Heatmap
    );
    handle_tui_mouse_event(&mut state, center_click, term_area);
    assert_eq!(
        state.effective_tui_center_view(),
        crate::state::TuiCenterView::SurroundEq
    );

    // Dragging the split border downward increases panel_split_ratio and keeps layout aligned;
    // dragging upward decreases it monotonically.
    let initial_split_y = layout.split_row_y;
    let initial_ratio = state.panel_split_ratio;
    let drag_down = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 20,
        row: initial_split_y + 2,
        modifiers: KeyModifiers::NONE,
    };
    handle_tui_mouse_event(&mut state, drag_down, term_area);
    assert!(
        state.panel_split_ratio > initial_ratio,
        "Dragging split border down must increase panel_split_ratio"
    );
    let down_layout = crate::ui::compute_tui_layout(term_area, &state, meta_len);
    assert_eq!(down_layout.split_row_y, initial_split_y + 2);

    let drag_up = MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 20,
        row: down_layout.split_row_y.saturating_sub(2),
        modifiers: KeyModifiers::NONE,
    };
    handle_tui_mouse_event(&mut state, drag_up, term_area);
    assert!(
        (state.panel_split_ratio - initial_ratio).abs() < 0.05,
        "Dragging split border back up must return near initial ratio"
    );
}

#[test]
fn test_cli_args_bench_windowed_fullscreen_and_help() {
    use clap::CommandFactory;

    // Bare --bench (alone or after positional file) defaults to Some(300)
    let bare_only =
        Args::try_parse_from(["rusttracker", "--bench"]).expect("Bare --bench alone must parse");
    assert_eq!(bare_only.bench, Some(300));

    let bare_trailing = Args::try_parse_from(["rusttracker", "song.mod", "--bench"])
        .expect("Trailing bare --bench must parse");
    assert_eq!(bare_trailing.bench, Some(300));
    assert_eq!(bare_trailing.file, vec!["song.mod".to_string()]);

    // Normalized CLI args also allow `--bench song.mod` without swallowing the filename
    let bare_leading = Args::try_parse_from(super::normalize_cli_args([
        std::ffi::OsString::from("rusttracker"),
        std::ffi::OsString::from("--bench"),
        std::ffi::OsString::from("song.mod"),
    ]))
    .expect("Normalized `--bench song.mod` must parse");
    assert_eq!(bare_leading.bench, Some(300));
    assert_eq!(bare_leading.file, vec!["song.mod".to_string()]);

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let non_utf8_path = std::ffi::OsString::from_vec(vec![
            b's', b'o', b'n', b'g', 0xff, b'.', b'm', b'o', b'd',
        ]);
        let normalized = super::normalize_cli_args([
            std::ffi::OsString::from("rusttracker"),
            std::ffi::OsString::from("--bench"),
            non_utf8_path.clone(),
        ]);
        assert_eq!(normalized[1], std::ffi::OsString::from("--bench=300"));
        assert_eq!(normalized[2], non_utf8_path);
    }

    // Explicit --bench <N> parses N
    let explicit_bench = Args::try_parse_from(super::normalize_cli_args([
        std::ffi::OsString::from("rusttracker"),
        std::ffi::OsString::from("--bench"),
        std::ffi::OsString::from("120"),
        std::ffi::OsString::from("song.mod"),
    ]))
    .expect("Explicit --bench 120 must parse");
    assert_eq!(explicit_bench.bench, Some(120));
    assert_eq!(explicit_bench.file, vec!["song.mod".to_string()]);

    // --windowed parses cleanly alone, and conflicts with --fullscreen
    let win_args =
        Args::try_parse_from(["rusttracker", "--windowed"]).expect("--windowed must parse");
    assert!(win_args.windowed);
    assert!(!win_args.fullscreen);
    assert!(
        Args::try_parse_from(["rusttracker", "--windowed", "--fullscreen"]).is_err(),
        "--windowed and --fullscreen must conflict"
    );

    // Removed placebo --gpu-fft flag must be rejected
    assert!(
        Args::try_parse_from(["rusttracker", "--gpu-fft"]).is_err(),
        "Removed placebo --gpu-fft flag must be rejected"
    );

    // Every flag in --help must have a non-empty description
    let mut cmd = Args::command();
    let help_text = cmd.render_help().to_string();
    for flag in [
        "--tui",
        "--mic",
        "--fullscreen",
        "--windowed",
        "--vis",
        "--list-vis",
        "--bench",
        "--uncapped",
        "--auto-vis-guard",
        "--stereo",
        "--passthrough",
        "--surround-upmix",
        "--no-surround-upmix",
        "--mpegh",
        "--no-mpegh",
        "--register-steam",
    ] {
        assert!(
            help_text.contains(flag),
            "Help output missing expected flag {flag}"
        );
    }
    assert!(!help_text.contains("--gpu-fft"));
}

#[test]
fn test_readme_cli_flags_and_env_vars_match_args_parser() {
    use clap::CommandFactory;

    let readme = include_str!("../README.md");
    let flags_header = "### Command-Line Flags";
    let env_header = "### Environment Variables";
    let after_flags = readme
        .split_once(flags_header)
        .expect("README.md must contain `### Command-Line Flags`")
        .1;
    let (flags_section, after_env_split) = after_flags
        .split_once(env_header)
        .expect("README.md must contain `### Environment Variables` after Command-Line Flags");
    let env_section = after_env_split
        .split_once("\n---")
        .map(|(s, _)| s)
        .unwrap_or(after_env_split);

    let sample_for_placeholder = |token: &str| -> &'static str {
        match token.trim() {
            "id" | "frames" => "120",
            "name" => "spectrum",
            "true" => "true",
            "false" => "false",
            _ => "test_value",
        }
    };

    let run_normalized_parse = |argv: &[&str]| -> Result<Args, clap::Error> {
        let os_args: Vec<std::ffi::OsString> = argv.iter().map(std::ffi::OsString::from).collect();
        Args::try_parse_from(super::normalize_cli_args(os_args))
    };

    // Extract every backticked `--...` span in `### Command-Line Flags`
    let mut documented_spans: Vec<String> = Vec::new();
    for line in flags_section.lines() {
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let after_open = &rest[start + 1..];
            let Some(end) = after_open.find('`') else {
                break;
            };
            let span = after_open[..end].trim();
            if span.starts_with("--") {
                documented_spans.push(span.to_string());
            }
            rest = &after_open[end + 1..];
        }
    }

    assert!(
        documented_spans.len() >= 14,
        "Expected at least 14 documented CLI flag spans in README.md, found {}",
        documented_spans.len()
    );

    let mut seen_flag_names = std::collections::HashSet::new();
    for span in &documented_spans {
        let parts: Vec<&str> = span.split_whitespace().collect();
        let flag_name = parts[0];
        seen_flag_names.insert(flag_name.to_string());

        match parts.get(1).copied() {
            None => {
                // Bare `--flag` invocation must always parse cleanly alone
                let parsed_alone = run_normalized_parse(&["rusttracker", flag_name])
                    .unwrap_or_else(|e| {
                        panic!("Documented flag `{span}` failed standalone parse: {e}")
                    });
                assert!(parsed_alone.file.is_empty());

                // Every bare flag (including bare `--vis`) with no value placeholder must NOT
                // swallow a trailing positional media filename.
                let parsed_with_file =
                    run_normalized_parse(&["rusttracker", flag_name, "song.mod"]).unwrap_or_else(
                        |e| panic!("Documented flag `{span}` failed parse before filename: {e}"),
                    );
                assert_eq!(
                    parsed_with_file.file,
                    vec!["song.mod".to_string()],
                    "Documented flag `{span}` swallowed `song.mod` instead of leaving it as sole positional file"
                );
            }
            Some(arg_spec) if arg_spec.starts_with('<') && arg_spec.ends_with('>') => {
                let inner = &arg_spec[1..arg_spec.len() - 1];
                for alt in inner.split('|') {
                    let sample = sample_for_placeholder(alt);
                    let parsed = run_normalized_parse(&[
                        "rusttracker",
                        flag_name,
                        sample,
                        "song.mod",
                    ])
                    .unwrap_or_else(|e| {
                        panic!("Documented flag `{span}` with value `{sample}` failed parse: {e}")
                    });
                    assert_eq!(
                        parsed.file,
                        vec!["song.mod".to_string()],
                        "Documented flag `{span}` with value `{sample}` treated value as positional file! (got {:?})",
                        parsed.file
                    );
                }
            }
            Some(arg_spec) if arg_spec.starts_with('[') && arg_spec.ends_with(']') => {
                let inner = &arg_spec[1..arg_spec.len() - 1];
                // Optional value omitted (both standalone and followed by positional filename)
                let parsed_bare =
                    run_normalized_parse(&["rusttracker", flag_name]).unwrap_or_else(|e| {
                        panic!("Documented optional flag `{span}` failed bare parse: {e}")
                    });
                assert!(parsed_bare.file.is_empty());

                let parsed_omitted_with_file = run_normalized_parse(&[
                    "rusttracker",
                    flag_name,
                    "song.mod",
                ])
                .unwrap_or_else(|e| {
                    panic!(
                        "Documented optional flag `{span}` failed bare parse before filename: {e}"
                    )
                });
                assert_eq!(
                    parsed_omitted_with_file.file,
                    vec!["song.mod".to_string()],
                    "Documented optional flag `{span}` swallowed `song.mod` when value omitted"
                );

                // Optional value provided explicitly
                for alt in inner.split('|') {
                    let sample = sample_for_placeholder(alt);
                    let parsed_explicit = run_normalized_parse(&[
                        "rusttracker",
                        flag_name,
                        sample,
                        "song.mod",
                    ])
                    .unwrap_or_else(|e| {
                        panic!(
                            "Documented optional flag `{span}` with `{sample}` failed parse: {e}"
                        )
                    });
                    assert_eq!(parsed_explicit.file, vec!["song.mod".to_string()]);
                }
            }
            Some(literal_val) => {
                // Literal example such as `--vis list`
                let parsed = run_normalized_parse(&["rusttracker", flag_name, literal_val])
                    .unwrap_or_else(|e| panic!("Documented invocation `{span}` failed parse: {e}"));
                assert!(parsed.file.is_empty());
            }
        }
    }

    // Reverse check: every `--long` flag and visible alias in `Args` must have non-empty
    // `--help` text and be documented in README.md
    let cmd = Args::command();
    for arg in cmd.get_arguments() {
        if arg.get_id() != "help" && arg.get_id() != "version" {
            assert!(
                arg.get_help()
                    .is_some_and(|h| !h.to_string().trim().is_empty()),
                "CLI argument `{}` in `struct Args` is missing `--help` description",
                arg.get_id()
            );
        }
        let Some(long) = arg.get_long() else {
            continue;
        };
        if long == "help" || long == "version" {
            continue;
        }
        let expected = format!("--{long}");
        assert!(
            seen_flag_names.contains(&expected),
            "CLI flag `{expected}` exists in `struct Args` (`src/main.rs`) but is missing from `README.md` Command-Line Flags"
        );
        if let Some(aliases) = arg.get_visible_aliases() {
            for alias in aliases {
                let expected_alias = format!("--{alias}");
                assert!(
                    seen_flag_names.contains(&expected_alias),
                    "Visible CLI alias `{expected_alias}` exists in `struct Args` but is missing from `README.md` Command-Line Flags"
                );
            }
        }
    }

    // Verify all user-facing RUSTTRACKER_* environment variables are documented in README.md,
    // and scan source files so no undocumented RUSTTRACKER_* env var can sneak in.
    for env_var in [
        "RUSTTRACKER_FPS_LIMIT",
        "RUSTTRACKER_PROFILE",
        "RUSTTRACKER_PRESENT_MODE",
        "RUSTTRACKER_PIPELINE_CACHE",
        "RUSTTRACKER_CACHE_DIR",
        "RUSTTRACKER_WAYLAND",
        "RUSTTRACKER_X11",
    ] {
        assert!(
            env_section.contains(env_var),
            "Environment variable `{env_var}` missing from `README.md` Environment Variables section"
        );
    }

    let root_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut env_scan_files = Vec::new();
    collect_rs_files(&root_dir.join("src"), &mut env_scan_files);
    collect_rs_files(
        &root_dir.join("crates/rt-render/src/engine"),
        &mut env_scan_files,
    );
    for file_path in env_scan_files {
        if file_path.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
            continue;
        }
        let file_label = file_path.display().to_string();
        let full_src = std::fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", file_path.display()));
        for needle in ["env::var(\"RUSTTRACKER_", "env::var_os(\"RUSTTRACKER_"] {
            let mut search = full_src.as_str();
            while let Some(pos) = search.find(needle) {
                let after_quote = &search[pos + needle.len() - "RUSTTRACKER_".len()..];
                if let Some(end_q) = after_quote.find('"') {
                    let var_name = &after_quote[..end_q];
                    // `RUSTTRACKER_APPDIR` is an internal AppImage mount path set by AppRun
                    if var_name != "RUSTTRACKER_APPDIR" {
                        assert!(
                            env_section.contains(var_name),
                            "Undocumented `{var_name}` read in `{file_label}` but missing from `README.md` Environment Variables"
                        );
                    }
                    search = &after_quote[end_q + 1..];
                } else {
                    break;
                }
            }
        }
    }
}

#[test]
fn test_cli_stereo_and_no_surround_upmix_flags() {
    // 1. `--stereo` forces 2.0 downmix, disables surround upmix + passthrough, and marks user_overridden
    let stereo_args =
        Args::try_parse_from(["rusttracker", "--stereo", "track.mkv"]).expect("--stereo");
    assert!(stereo_args.stereo);
    assert!(super::has_cli_upmix_override(&stereo_args));
    assert_eq!(stereo_args.file, vec!["track.mkv".to_string()]);
    let mut stereo_state = AppState::new("track.mkv".to_string());
    stereo_state.passthrough_enabled = true;
    super::apply_cli_audio_flags(&mut stereo_state, &stereo_args);
    assert!(stereo_state.force_stereo_downmix);
    assert!(!stereo_state.surround_upmix.enabled);
    assert!(stereo_state.surround_upmix.user_overridden);
    assert!(!stereo_state.passthrough_enabled);
    assert_eq!(
        stereo_state.osd_text.as_deref(),
        Some("🎧 Stereo Downmix (2.0): Forced")
    );

    // `--stereo --no-mpegh` and `--stereo --no-surround-upmix` must keep the stereo OSD
    let stereo_no_mpegh =
        Args::try_parse_from(["rusttracker", "--stereo", "--no-mpegh", "track.mkv"])
            .expect("--stereo --no-mpegh");
    let mut stereo_no_mpegh_state = AppState::new("track.mkv".to_string());
    super::apply_cli_audio_flags(&mut stereo_no_mpegh_state, &stereo_no_mpegh);
    assert!(stereo_no_mpegh_state.force_stereo_downmix);
    assert!(!stereo_no_mpegh_state.surround_upmix.mpegh_bitstream);
    assert_eq!(
        stereo_no_mpegh_state.osd_text.as_deref(),
        Some("🎧 Stereo Downmix (2.0): Forced")
    );

    // Runtime `set_force_stereo_downmix` preserves stereo OSD across `reconcile_passthrough_change_osd`
    let mut runtime_stereo = AppState::new("track.mkv".to_string());
    runtime_stereo.file_loaded = true;
    runtime_stereo.hardware_channels = 8;
    runtime_stereo.passthrough_enabled = true;
    runtime_stereo.set_force_stereo_downmix(true);
    assert!(runtime_stereo.force_stereo_downmix);
    assert!(!runtime_stereo.passthrough_enabled);
    assert!(!runtime_stereo.surround_upmix.enabled);
    assert!(runtime_stereo.surround_upmix.user_overridden);
    assert_eq!(runtime_stereo.passthrough_change_request, Some(false));
    super::reconcile_passthrough_change_osd(&mut runtime_stereo, false);
    assert_eq!(
        runtime_stereo.osd_text.as_deref(),
        Some("🎧 Stereo Downmix (2.0): Forced")
    );
    runtime_stereo.set_force_stereo_downmix(false);
    assert!(!runtime_stereo.force_stereo_downmix);
    assert!(!runtime_stereo.surround_upmix.user_overridden);
    super::reconcile_passthrough_change_osd(&mut runtime_stereo, false);
    assert_eq!(
        runtime_stereo.osd_text.as_deref(),
        Some("🎧 Stereo Downmix: OFF")
    );

    // 2. `--no-surround-upmix` and `--no-upmix` disable automatic upmixing cleanly
    let no_upmix_args = Args::try_parse_from(["rusttracker", "--no-surround-upmix", "song.flac"])
        .expect("--no-surround-upmix");
    assert!(no_upmix_args.no_surround_upmix);
    assert!(!no_upmix_args.surround_upmix);
    assert!(super::has_cli_upmix_override(&no_upmix_args));
    assert_eq!(no_upmix_args.file, vec!["song.flac".to_string()]);

    let no_upmix_alias =
        Args::try_parse_from(["rusttracker", "--no-upmix", "song.flac"]).expect("--no-upmix");
    assert!(no_upmix_alias.no_surround_upmix);

    let mut no_upmix_state = AppState::new("song.flac".to_string());
    assert!(no_upmix_state.surround_upmix.enabled);
    super::apply_cli_audio_flags(&mut no_upmix_state, &no_upmix_args);
    assert!(!no_upmix_state.surround_upmix.enabled);
    assert!(no_upmix_state.surround_upmix.user_overridden);
    assert!(!no_upmix_state.force_stereo_downmix);

    // 3. `--mpegh` preserves `has_cli_upmix_override` and keeps 11.1 OSD; `--no-surround-upmix --mpegh`
    // allows native >8ch MPEG-H bitstreaming without re-enabling upmix
    let mpegh_only_args =
        Args::try_parse_from(["rusttracker", "--mpegh", "song.flac"]).expect("--mpegh");
    assert!(super::has_cli_upmix_override(&mpegh_only_args));
    let mut mpegh_only_state = AppState::new("song.flac".to_string());
    super::apply_cli_audio_flags(&mut mpegh_only_state, &mpegh_only_args);
    if cfg!(any(target_os = "windows", target_os = "linux")) {
        assert_eq!(
            mpegh_only_state.osd_text.as_deref(),
            Some("🔊 11.1 (7.1.4 Spatial Audio / MPEG-H 3D): ON")
        );
    }

    // `--surround-upmix` updates pre-load "Armed" OSD to "ON (<badge>)" once >2ch stream opens
    let upmix_args = Args::try_parse_from(["rusttracker", "--surround-upmix", "song.flac"])
        .expect("--surround-upmix");
    let mut upmix_state = AppState::new("song.flac".to_string());
    super::apply_cli_audio_flags(&mut upmix_state, &upmix_args);
    upmix_state.hardware_channels = 8;
    upmix_state.source_channels = 2;
    super::reconcile_post_load_cli_audio_osd(&mut upmix_state, &upmix_args);
    assert_eq!(
        upmix_state.osd_text.as_deref(),
        Some("🔊 Surround Upmix: ON (7.1 Surround — Bed Only)")
    );

    let native_mpegh_args = Args::try_parse_from([
        "rusttracker",
        "--no-surround-upmix",
        "--mpegh",
        "bed714.wav",
    ])
    .expect("--no-surround-upmix --mpegh");
    assert!(super::has_cli_upmix_override(&native_mpegh_args));
    let mut native_mpegh_state = AppState::new("bed714.wav".to_string());
    super::apply_cli_audio_flags(&mut native_mpegh_state, &native_mpegh_args);
    assert!(!native_mpegh_state.surround_upmix.enabled);
    if cfg!(any(target_os = "windows", target_os = "linux")) {
        assert!(native_mpegh_state.surround_upmix.mpegh_bitstream);
    }

    // 4. Mutually contradictory audio mode flags are rejected by clap
    for conflicting in [
        &["rusttracker", "--stereo", "--passthrough"][..],
        &["rusttracker", "--stereo", "--surround-upmix"][..],
        &["rusttracker", "--stereo", "--upmix"][..],
        &["rusttracker", "--stereo", "--mpegh"][..],
        &["rusttracker", "--surround-upmix", "--no-surround-upmix"][..],
        &["rusttracker", "--upmix", "--no-upmix"][..],
        &["rusttracker", "--mpegh", "--no-mpegh"][..],
        &["rusttracker", "--passthrough", "--surround-upmix"][..],
        &["rusttracker", "--passthrough", "--mpegh"][..],
    ] {
        assert!(
            Args::try_parse_from(conflicting).is_err(),
            "Expected conflicting flags {conflicting:?} to be rejected"
        );
    }
}

#[test]
fn test_cli_vis_and_bench_normalization_and_x11_wayland_precedence() {
    let parse_norm = |argv: &[&str]| -> Result<Args, clap::Error> {
        let os_args: Vec<std::ffi::OsString> = argv.iter().map(std::ffi::OsString::from).collect();
        Args::try_parse_from(super::normalize_cli_args(os_args))
    };

    // Bare `--vis` before a media path, URL, or `--` lists visualizers without swallowing the file
    for (argv, expected_file) in [
        (&["rusttracker", "--vis", "song.mod"][..], "song.mod"),
        (
            &["rusttracker", "--vis", "audio_tests/ive_got_the_power.mod"][..],
            "audio_tests/ive_got_the_power.mod",
        ),
        (
            &["rusttracker", "--vis", "https://example.com/live.ogg"][..],
            "https://example.com/live.ogg",
        ),
        (
            &["rusttracker", "--vis", "--", "-song.mod"][..],
            "-song.mod",
        ),
    ] {
        let parsed = parse_norm(argv).unwrap_or_else(|e| panic!("{argv:?} failed: {e}"));
        assert_eq!(parsed.vis.as_deref(), Some(""));
        assert_eq!(parsed.file, vec![expected_file.to_string()]);
    }

    // Explicit visualizer names/IDs and unknown non-path tokens still bind to `--vis`
    let vis_named = parse_norm(&["rusttracker", "--vis", "spectrum", "song.mod"]).unwrap();
    assert_eq!(vis_named.vis.as_deref(), Some("spectrum"));
    assert_eq!(vis_named.file, vec!["song.mod".to_string()]);

    let vis_unknown = parse_norm(&["rusttracker", "--vis", "nonexistent_vis", "song.mod"])
        .expect("unknown vis token");
    assert_eq!(vis_unknown.vis.as_deref(), Some("nonexistent_vis"));
    assert_eq!(vis_unknown.file, vec!["song.mod".to_string()]);

    // `--bench` before `--` defaults to 300, whereas overflowing numeric frame counts fail cleanly
    let bench_sep = parse_norm(&["rusttracker", "--bench", "--", "-song.mod"]).unwrap();
    assert_eq!(bench_sep.bench, Some(300));
    assert_eq!(bench_sep.file, vec!["-song.mod".to_string()]);
    assert!(
        parse_norm(&["rusttracker", "--bench", "99999999999999999999", "song.mod"]).is_err(),
        "Overflowing numeric `--bench` count must fail clap u32 validation instead of becoming a filename"
    );

    // `RUSTTRACKER_X11=1` always overrides `RUSTTRACKER_WAYLAND=1`
    assert!(super::should_prefer_x11_backend(
        Some("1"),
        Some("1"),
        true,
        true
    ));
    // Explicit `RUSTTRACKER_WAYLAND=1` or default with active `WAYLAND_DISPLAY` uses native Wayland
    assert!(!super::should_prefer_x11_backend(
        Some("1"),
        None,
        true,
        true
    ));
    assert!(!super::should_prefer_x11_backend(None, None, true, true));
    assert!(!super::should_prefer_x11_backend(None, None, true, false));
    // `RUSTTRACKER_WAYLAND=0` / `false` forces X11 when DISPLAY is available
    assert!(super::should_prefer_x11_backend(
        Some("0"),
        None,
        true,
        true
    ));
    assert!(super::should_prefer_x11_backend(
        Some("false"),
        None,
        true,
        true
    ));
    // Falls back to X11 when WAYLAND_DISPLAY is absent and DISPLAY is present
    assert!(super::should_prefer_x11_backend(None, None, false, true));
    assert!(!super::should_prefer_x11_backend(None, None, false, false));

    // Canonical visualizer ID lookup must NOT collide with array index
    // index 1 has id=25 ("Studio LED Spectrum"), index 3 has id=1 ("CRT Oscilloscope")
    let idx_id1 = super::matches_visualizer_spec("1").expect("--vis 1 must match id=1");
    assert_eq!(crate::state::VISUALIZERS[idx_id1].id, 1);
    let idx_id25 = super::matches_visualizer_spec("25").expect("--vis 25 must match id=25");
    assert_eq!(crate::state::VISUALIZERS[idx_id25].id, 25);
    assert!(
        super::matches_visualizer_spec("9999").is_none(),
        "Unknown visualizer ID must return None"
    );
}

#[test]
fn test_shortcuts_vdf_upsert_preserves_prior_entries_and_tags() {
    // Build a synthetic Steam shortcuts.vdf with an existing non-RustTracker shortcut
    // that includes 0x03 (float), 0x07 (u64), and a nested `tags` map closed with 0x08.
    let mut vdf = Vec::new();
    vdf.extend_from_slice(b"\x00shortcuts\x00");
    vdf.extend_from_slice(b"\x000\x00");
    vdf.extend_from_slice(b"\x02appid\x00");
    vdf.extend_from_slice(&123456u32.to_le_bytes());
    vdf.extend_from_slice(b"\x01AppName\x00OtherGame\x00");
    vdf.extend_from_slice(b"\x01Exe\x00\"/usr/bin/other\"\x00");
    vdf.extend_from_slice(b"\x03score\x00");
    vdf.extend_from_slice(&1.5f32.to_le_bytes());
    vdf.extend_from_slice(b"\x07bigid\x00");
    vdf.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    vdf.extend_from_slice(b"\x00tags\x00\x010\x00favorite\x00\x08");
    vdf.push(0x08); // close entry 0
    vdf.push(0x08); // close root shortcuts map
    vdf.push(0x08); // trailing EOF

    let (updated, rt_appid) = super::upsert_rusttracker_shortcut_in_vdf(
        &vdf,
        "/opt/rusttracker/rusttracker",
        "/opt/rusttracker/",
        "/opt/rusttracker/icon.png",
    );

    // Prior entry's bytes up through its entry terminator (0x08) must remain completely intact!
    let prior_entry_end = vdf.len() - 2;
    assert_eq!(
        &updated[..prior_entry_end],
        &vdf[..prior_entry_end],
        "Inserting a new shortcut must never strip inner tags or terminators of prior entries"
    );
    // Must end with closing root map 0x08 and EOF 0x08
    assert!(updated.ends_with(b"\x08\x08\x08\x08")); // tags end, entry 1 end, root end, EOF
    let found = super::extract_rusttracker_shortcut_appids(&updated);
    assert_eq!(found, vec![rt_appid]);
}

#[test]
fn test_gamepad_l2_r2_scrub_no_hud_collision_and_l3_r3_bindings() {
    let mut state = AppState::new("ScrubTest.mod".to_string());
    state.file_loaded = true;
    state.duration_seconds = 120.0;
    state.current_seconds = 60.0;
    state.show_hud = true;
    state.hardware_channels = 8;
    state.source_channels = 2;

    // 1. Pulling L2 (analog + digital LeftTrigger2 event from gilrs) scrubs backward and NEVER hides HUD
    assert!(
        !super::apply_gamepad_playback_button(&mut state, gilrs::Button::LeftTrigger2),
        "Digital LeftTrigger2 must not trigger a button action"
    );
    assert!(state.show_hud, "LeftTrigger2 must never toggle HUD");
    assert!(super::apply_gamepad_trigger_scrub(
        &mut state,
        0.8,
        0.0,
        1.0 / 60.0
    ));
    assert!(
        state.current_seconds < 60.0,
        "L2 analog pull must scrub backward"
    );
    assert!(state.seek_request.is_some());
    assert!(state.show_hud, "L2 analog scrub must leave HUD visible");

    // 2. Pulling R2 (analog + digital RightTrigger2 event) scrubs forward and NEVER hides HUD
    let after_l2 = state.current_seconds;
    assert!(!super::apply_gamepad_playback_button(
        &mut state,
        gilrs::Button::RightTrigger2
    ));
    assert!(super::apply_gamepad_trigger_scrub(
        &mut state,
        0.0,
        1.0,
        1.0 / 60.0
    ));
    assert!(state.current_seconds > after_l2);
    assert!(state.show_hud);

    // 3. Deadzone and modal/unloaded guards prevent spurious scrubbing or seek OSDs
    state.seek_request = None;
    assert!(!super::apply_gamepad_trigger_scrub(
        &mut state,
        0.02,
        0.0,
        1.0 / 60.0
    ));
    assert!(state.seek_request.is_none());
    state.show_vis_picker = true;
    assert!(!super::apply_gamepad_trigger_scrub(
        &mut state,
        1.0,
        0.0,
        1.0 / 60.0
    ));
    assert!(!super::apply_gamepad_playback_button(
        &mut state,
        gilrs::Button::DPadRight
    ));
    state.show_vis_picker = false;

    let mut unloaded = AppState::new(String::new());
    for seek_btn in [
        gilrs::Button::DPadLeft,
        gilrs::Button::DPadRight,
        gilrs::Button::LeftTrigger,
        gilrs::Button::RightTrigger,
    ] {
        assert!(
            !super::apply_gamepad_playback_button(&mut unloaded, seek_btn),
            "Unloaded state must ignore {seek_btn:?} seek without false OSD"
        );
        assert!(unloaded.seek_request.is_none());
        assert!(unloaded.osd_text.is_none());
    }

    // 4. L3 (LeftThumb) toggles HUD visibility cleanly
    state.video_mode = 3;
    assert!(super::apply_gamepad_playback_button(
        &mut state,
        gilrs::Button::LeftThumb
    ));
    assert!(!state.show_hud);
    assert_eq!(state.video_mode, 0);
    assert!(super::apply_gamepad_playback_button(
        &mut state,
        gilrs::Button::LeftThumb
    ));
    assert!(state.show_hud);

    // 5. R3 (RightThumb) toggles Surround Upmix cleanly without affecting L3 HUD role
    let initial_upmix = state.surround_upmix.enabled;
    assert!(super::apply_gamepad_playback_button(
        &mut state,
        gilrs::Button::RightThumb
    ));
    assert_eq!(state.surround_upmix.enabled, !initial_upmix);
    assert!(super::apply_gamepad_playback_button(
        &mut state,
        gilrs::Button::RightThumb
    ));
    assert_eq!(state.surround_upmix.enabled, initial_upmix);

    // 6. Verify env_flag_enabled handles truthy/falsy/empty values accurately
    assert!(super::env_flag_enabled(Some("1")));
    assert!(super::env_flag_enabled(Some("true")));
    assert!(super::env_flag_enabled(Some("TRUE")));
    assert!(super::env_flag_enabled(Some(" 1 ")));
    assert!(!super::env_flag_enabled(Some("0")));
    assert!(!super::env_flag_enabled(Some("false")));
    assert!(!super::env_flag_enabled(Some("FALSE")));
    assert!(!super::env_flag_enabled(Some("")));
    assert!(!super::env_flag_enabled(Some("   ")));
    assert!(!super::env_flag_enabled(None));

    // 7. Verify UI overlays, README, VIS_REVIEW, and THIRD_PARTY_NOTICES match live ground truth
    let engine_src = concat_dir_rs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/rt-render/src/engine"),
        true,
    );
    assert!(!engine_src.contains("\"L2\", \"Toggle HUD\""));
    assert!(!engine_src.contains("Some(\"L2\"), \"Toggle HUD\""));
    assert!(!engine_src.contains("\"q / esc\""));
    assert!(!engine_src.contains("Surround Upmix & EQ"));

    let state_src = include_str!("../crates/rt-core/src/state.rs");
    let shader_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let mut total_wgsl = 0usize;
    let mut standalone_wgsl = 0usize;
    for entry in std::fs::read_dir(&shader_dir).expect("read src/shaders") {
        let path = entry.expect("dir entry").path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("wgsl") {
            total_wgsl += 1;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('_') {
                standalone_wgsl += 1;
                assert!(
                    engine_src.contains(name) || state_src.contains(name),
                    "Standalone shader {name} in src/shaders/ is not referenced in engine.rs or state.rs"
                );
            }
        }
    }
    let shared_wgsl = total_wgsl - standalone_wgsl;
    assert_eq!(shared_wgsl, 2);
    assert_eq!(standalone_wgsl, 30);
    assert_eq!(total_wgsl, 32);

    let vis_count = crate::state::VISUALIZERS.len();
    let vis_review = include_str!("../VIS_REVIEW.md");
    assert!(vis_review.contains("Document Status Note:"));
    assert!(vis_review.contains(&format!("{vis_count}` registered visualizers")));
    assert!(vis_review.contains(&format!("{total_wgsl}` `.wgsl` files")));
    assert!(vis_review.contains(&format!(
        "{standalone_wgsl}` standalone Naga-validated shader modules"
    )));
    assert!(vis_review.contains(&format!(
        "- {standalone_wgsl} standalone WGSL shader modules + {shared_wgsl} shared include headers (`_common.wgsl`, `_glyph_font.wgsl`) = {total_wgsl} `.wgsl` files"
    )));
    assert!(!vis_review.contains("51` `.wgsl` files"));
    assert!(!vis_review.contains("49 standalone WGSL"));

    let notices = include_str!("../THIRD_PARTY_NOTICES.txt");
    assert!(notices.contains("MPEG-H 3D Audio Low Complexity Profile Encoder"));
    assert!(!notices.contains("Baseline Profile Encoder"));
    assert!(notices.contains("[third_party/libmpeghe/LICENSE]"));
    assert!(notices.contains("[third_party/libmpeghe/LICENSE2]"));
}

#[test]
fn test_license_notices_and_packaging_manifests_invariants() {
    let root_license = include_str!("../LICENSE").replace("\r\n", "\n");
    let notices = include_str!("../THIRD_PARTY_NOTICES.txt").replace("\r\n", "\n");
    let libmpeghe_license = include_str!("../third_party/libmpeghe/LICENSE").replace("\r\n", "\n");
    let libmpeghe_license2 =
        include_str!("../third_party/libmpeghe/LICENSE2").replace("\r\n", "\n");

    // 1. Verify GPLv3 §7 Additional Permission and verbatim Ittiam LICENSE + LICENSE2 reproduction
    assert!(root_license.contains(
        "Additional Permission Under GNU GPL Version 3 Section 7 (`libmpeghe` Exception)"
    ));
    assert!(root_license.contains("THIRD_PARTY_NOTICES.txt"));
    assert!(notices.contains("MPEG-H 3D Audio Low Complexity Profile Encoder"));
    assert!(!notices.contains("Baseline Profile Encoder"));
    assert!(
        notices.contains(libmpeghe_license.trim()),
        "THIRD_PARTY_NOTICES.txt must reproduce third_party/libmpeghe/LICENSE verbatim"
    );
    assert!(
        notices.contains(libmpeghe_license2.trim()),
        "THIRD_PARTY_NOTICES.txt must reproduce third_party/libmpeghe/LICENSE2 verbatim"
    );

    // 2. Verify all 6 distribution manifests bundle LICENSE and THIRD_PARTY_NOTICES.txt
    let cargo_toml = include_str!("../Cargo.toml");
    assert!(
        cargo_toml.contains(r#"dest = "/usr/share/licenses/rusttracker/LICENSE""#)
            && cargo_toml
                .contains(r#"dest = "/usr/share/licenses/rusttracker/THIRD_PARTY_NOTICES.txt""#),
        "Cargo.toml [package.metadata.generate-rpm] must bundle LICENSE and THIRD_PARTY_NOTICES.txt"
    );
    assert!(
        cargo_toml.contains(r#"["LICENSE", "usr/share/doc/rusttracker/", "644"]"#)
            && cargo_toml
                .contains(r#"["THIRD_PARTY_NOTICES.txt", "usr/share/doc/rusttracker/", "644"]"#),
        "Cargo.toml [package.metadata.deb] must bundle LICENSE and THIRD_PARTY_NOTICES.txt"
    );

    let repackage = include_str!("../repackage_appimage.sh");
    assert!(
        repackage.contains("cp LICENSE THIRD_PARTY_NOTICES.txt AppDir/usr/share/doc/rusttracker/")
            && repackage.contains(
                "cp LICENSE THIRD_PARTY_NOTICES.txt AppDir/usr/share/licenses/rusttracker/"
            ),
        "repackage_appimage.sh must install LICENSE and THIRD_PARTY_NOTICES.txt into AppDir"
    );

    let installer_iss = include_str!("../installer.iss");
    assert!(
        installer_iss.contains(r#"Excludes: "LICENSE,THIRD_PARTY_NOTICES.txt""#),
        "installer.iss must exclude LICENSE and THIRD_PARTY_NOTICES.txt from release\\* wildcard to prevent duplicate entries"
    );
    assert!(
        installer_iss.contains(r#"Source: "LICENSE"; DestDir: "{app}""#)
            && installer_iss.contains(r#"Source: "THIRD_PARTY_NOTICES.txt"; DestDir: "{app}""#),
        "installer.iss must bundle LICENSE and THIRD_PARTY_NOTICES.txt into {{app}}"
    );

    let win_yml = include_str!("../.github/workflows/windows-build.yml");
    assert!(
        win_yml.contains("cp LICENSE THIRD_PARTY_NOTICES.txt release/"),
        "windows-build.yml must stage LICENSE and THIRD_PARTY_NOTICES.txt into release/"
    );

    let macos_yml = include_str!("../.github/workflows/macos.yml");
    assert!(
        macos_yml.contains(
            "cp assets/soundfont.sf2 LICENSE THIRD_PARTY_NOTICES.txt RustTracker.app/Contents/Resources/"
        ),
        "macos.yml must bundle LICENSE and THIRD_PARTY_NOTICES.txt into RustTracker.app/Contents/Resources/"
    );

    let android_gradle = include_str!("../android/app/build.gradle.kts");
    assert!(
        android_gradle
            .contains(r#"File(rootDir, "LICENSE"), File(rootDir, "THIRD_PARTY_NOTICES.txt")"#)
            && android_gradle.contains("addGeneratedSourceDirectory(")
            && android_gradle.contains("SyncLicenseAssetsTask::outputDir"),
        "android/app/build.gradle.kts must wire SyncLicenseAssetsTask via AGP addGeneratedSourceDirectory"
    );
}

#[test]
fn test_phase3_workspace_crates_and_openmpt_direct_ffi_invariants() {
    let root_toml = include_str!("../Cargo.toml");
    assert!(
        root_toml.contains("[workspace]"),
        "Root Cargo.toml must define [workspace]"
    );
    for member in [
        "crates/rt-core",
        "crates/rt-decode",
        "crates/rt-audio",
        "crates/rt-render",
        "crates/rt-tui",
        "crates/rt-tools",
    ] {
        assert!(
            root_toml.contains(member),
            "Workspace member {member} missing from root Cargo.toml"
        );
        let sub_toml_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(member)
            .join("Cargo.toml");
        let sub_toml = std::fs::read_to_string(&sub_toml_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", sub_toml_path.display()));
        assert!(
            !sub_toml.contains("openmpt =") && !sub_toml.contains("openmpt-sys"),
            "{} must not depend on unmaintained openmpt / openmpt-sys crates",
            sub_toml_path.display()
        );
    }
    assert!(
        !root_toml.contains("openmpt =") && !root_toml.contains("openmpt-sys"),
        "Root Cargo.toml must not depend on unmaintained openmpt / openmpt-sys crates"
    );

    // Ensure Linux active-sink monitoring in rt-audio never spawns wpctl/pactl subprocesses
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let audio_prod = concat_dir_rs(&manifest_dir.join("crates/rt-audio/src"), true);
    assert!(
        !audio_prod.contains("Command::new(\"wpctl\")")
            && !audio_prod.contains("Command::new(\"pactl\")"),
        "crates/rt-audio/src must use non-spawning in-process Linux audio topology monitoring"
    );

    // Ensure zero hidden dead_code/unused/deprecated allow attributes across all workspace Rust files
    let mut all_rs = Vec::new();
    collect_rs_files(&manifest_dir.join("src"), &mut all_rs);
    collect_rs_files(&manifest_dir.join("crates"), &mut all_rs);
    assert!(
        !all_rs.is_empty(),
        "Recursive Rust file scan must cover workspace sources"
    );
    for full_path in all_rs {
        if full_path.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
            continue;
        }
        let content = std::fs::read_to_string(&full_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", full_path.display()));
        let prod = content.split("#[cfg(test)]").next().unwrap_or(&content);
        for forbidden in ["allow(dead_code", "allow(unused", "allow(deprecated"] {
            assert!(
                !prod.contains(forbidden),
                "Forbidden warning suppression `{forbidden}` found in {}",
                full_path.display()
            );
        }
    }
}

#[test]
fn test_vis_review_flip_water_and_package_metadata_truthfulness() {
    let vis_review = include_str!("../VIS_REVIEW.md");
    let flip_water_wgsl = include_str!("shaders/flip_water_compute.wgsl");
    let water_splash_wgsl = include_str!("shaders/vis_water_splash.wgsl");

    assert!(flip_water_wgsl.contains("const SOR_OMEGA: f32 = 0.75;"));
    assert!(flip_water_wgsl.contains("fn cs_pressure_0_to_1("));
    assert!(flip_water_wgsl.contains("fn cs_pressure_1_to_0("));
    assert!(flip_water_wgsl.contains("fn cs_g2p_advect_splat("));
    assert!(flip_water_wgsl.contains("splash_compact.active_indices[slot] = p_idx;"));
    assert!(water_splash_wgsl.contains("fn vs_splash("));
    assert!(water_splash_wgsl.contains("splash_compact.active_indices[inst_id]"));

    assert!(vis_review.contains("`SOR_OMEGA = 0.75`"));
    assert!(vis_review.contains("`cs_pressure_0_to_1` / `cs_pressure_1_to_0`"));
    assert!(vis_review.contains("`cs_g2p_advect_splat`"));
    assert!(vis_review.contains("`splash_compact.active_indices`"));
    assert!(vis_review.contains("`vs_splash`"));
    assert!(!vis_review.contains("`flip_pressure`"));
    assert!(!vis_review.contains(r"$\omega = 1.5$"));

    let cargo_toml = include_str!("../Cargo.toml");
    assert!(cargo_toml.contains(r#"authors = ["Soddentrough"]"#));
    assert!(cargo_toml.contains(r#"copyright = "2026, Soddentrough""#));
    assert!(
        cargo_toml
            .contains(r#"description = "High-Performance WGPU Audio, Video & Tracker Visualizer""#)
    );
    assert!(cargo_toml.contains(
        r#"extended-description = "A high-performance, real-time audio visualizer, video player, and tracker module player.""#
    ));
    assert!(!cargo_toml.contains("Vulkan Tracker Module Visualizer"));

    let metainfo = include_str!("../com.soddentrough.rusttracker.metainfo.xml");
    assert!(metainfo.contains(
        "<summary>High-Performance WGPU Audio, Video &amp; Tracker Visualizer</summary>"
    ));
    assert!(!metainfo.contains("Vulkan Tracker Module Visualizer"));

    let desktop = include_str!("../rusttracker.desktop");
    assert!(desktop.contains("Comment=High-Performance WGPU Audio, Video & Tracker Visualizer"));
}

#[test]
fn test_open_file_and_ipc_unpause_and_clear_track_ended() {
    // 1. Opening a new file via native file picker after the previous track ended & paused
    // must clear `is_paused`, clear `track_ended` (both state & `rt_control` latch), and queue `load_request`.
    let mut state = AppState::new("Previous.flac".to_string());
    state.file_loaded = true;
    state.playlist = std::sync::Arc::new(vec!["C:\\Music\\Previous.flac".to_string()]);
    state.playlist_index = 0;
    state.track_ended = true;
    state.is_paused = true;
    state.rt_control.signal_track_ended();
    state.sync_rt_control();
    assert!(state.rt_control.is_paused());

    let mut initial_dir = std::path::PathBuf::from("C:\\");
    let picked = "C:\\Music\\Howard Jones\\01 - Things Can Only Get Better.flac".to_string();
    assert!(apply_picked_file_paths_to_state(
        &mut state,
        vec![picked.clone()],
        &mut initial_dir
    ));
    assert_eq!(
        initial_dir,
        std::path::PathBuf::from("C:\\Music\\Howard Jones")
    );
    assert_eq!(state.load_request.as_deref(), Some(picked.as_str()));
    assert!(state.file_loaded);
    assert!(!state.is_paused);
    assert!(!state.rt_control.is_paused());
    assert!(!state.track_ended);
    assert!(!state.rt_control.take_track_ended());

    // 2. Appending a file after a single-track playlist has finished and advancing on `track_ended`
    // must unpause and load the newly appended track.
    state.load_request = None;
    state.append_to_playlist = true;
    state.track_ended = true;
    state.is_paused = true;
    state.rt_control.signal_track_ended();
    state.sync_rt_control();

    let appended = "C:\\Music\\Howard Jones\\02 - What Is Love.flac".to_string();
    assert!(apply_picked_file_paths_to_state(
        &mut state,
        vec![appended.clone()],
        &mut initial_dir
    ));
    assert_eq!(state.playlist.len(), 2);
    assert!(advance_playlist_on_track_ended(&mut state));
    state.sync_rt_control();
    assert_eq!(state.playlist_index, 1);
    assert_eq!(state.load_request.as_deref(), Some(appended.as_str()));
    assert!(!state.is_paused);
    assert!(!state.rt_control.is_paused());
    assert!(!state.track_ended);
    assert!(!state.rt_control.take_track_ended());

    // 3. IPC open-files path: fresh load from unloaded/paused state must unpause and clear `track_ended`.
    let mut ipc_state = AppState::new(String::new());
    ipc_state.track_ended = true;
    ipc_state.is_paused = true;
    ipc_state.rt_control.signal_track_ended();
    ipc_state.sync_rt_control();

    let ipc_first = "C:\\Music\\TrackA.flac".to_string();
    assert!(apply_ipc_file_paths_to_state(
        &mut ipc_state,
        vec![ipc_first.clone()]
    ));
    assert_eq!(ipc_state.load_request.as_deref(), Some(ipc_first.as_str()));
    assert!(ipc_state.file_loaded);
    assert!(!ipc_state.is_paused);
    assert!(!ipc_state.rt_control.is_paused());
    assert!(!ipc_state.track_ended);
    assert!(!ipc_state.rt_control.take_track_ended());

    // 4. IPC open-files path when appending to an ended & paused session must also unpause & clear `track_ended`.
    ipc_state.load_request = None;
    ipc_state.track_ended = true;
    ipc_state.is_paused = true;
    ipc_state.rt_control.signal_track_ended();
    ipc_state.sync_rt_control();

    let ipc_second = "C:\\Music\\TrackB.flac".to_string();
    assert!(apply_ipc_file_paths_to_state(
        &mut ipc_state,
        vec![ipc_second.clone()]
    ));
    assert_eq!(ipc_state.playlist_index, 1);
    assert_eq!(ipc_state.load_request.as_deref(), Some(ipc_second.as_str()));
    assert!(!ipc_state.is_paused);
    assert!(!ipc_state.rt_control.is_paused());
    assert!(!ipc_state.track_ended);
    assert!(!ipc_state.rt_control.take_track_ended());
}
