use crate::state::AppState;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Args {
    #[arg(
        value_name = "FILE_OR_URL",
        help = "Media file(s), tracker module(s), playlist(s), folder(s), or stream URL(s) to open"
    )]
    pub(crate) file: Vec<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Launch in terminal user interface (TUI) mode instead of the Vulkan/wgpu GUI"
    )]
    pub(crate) tui: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Capture live audio from the system microphone / loopback input device"
    )]
    pub(crate) mic: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "windowed",
        help = "Launch in borderless fullscreen mode"
    )]
    pub(crate) fullscreen: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "fullscreen",
        help = "Force windowed mode (overrides automatic fullscreen under Gamescope)"
    )]
    pub(crate) windowed: bool,

    #[arg(
        long,
        value_name = "ID_OR_NAME",
        num_args = 0..=1,
        default_missing_value = "",
        help = "Start directly on the specified visualizer ID or short name (pass empty or 'list' to list visualizers)"
    )]
    pub(crate) vis: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "List all available visualizers with their IDs, short names, and descriptions, then exit"
    )]
    pub(crate) list_vis: bool,

    #[arg(
        long,
        value_name = "FRAMES",
        num_args = 0..=1,
        default_missing_value = "300",
        help = "Run automated benchmark for FRAMES frames (default: 300) and print telemetry"
    )]
    pub(crate) bench: Option<u32>,

    #[arg(
        long,
        default_value_t = false,
        help = "Disable VSync / display sync and unlock uncapped frame rates"
    )]
    pub(crate) uncapped: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Let the frame-budget watchdog automatically uncheck and switch away from heavy visualizers that run below 85% of the refresh rate (off by default; visualizers only change on user input)"
    )]
    pub(crate) auto_vis_guard: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["passthrough", "surround_upmix", "mpegh"],
        help = "Force 2.0 stereo downmix output even on multi-channel hardware endpoints"
    )]
    pub(crate) stereo: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["stereo", "surround_upmix", "mpegh"],
        help = "Enable hardware bitstream passthrough (WASAPI Exclusive / PipeWire) for compressed surround formats"
    )]
    pub(crate) passthrough: bool,

    #[arg(
        long,
        visible_alias = "upmix",
        default_value_t = false,
        conflicts_with_all = ["no_surround_upmix", "stereo", "passthrough"],
        help = "Enable automatic 7.1.4 surround & height upmixing with configurable crossover EQ"
    )]
    pub(crate) surround_upmix: bool,

    #[arg(
        long,
        visible_alias = "no-upmix",
        default_value_t = false,
        conflicts_with = "surround_upmix",
        help = "Disable automatic surround & height upmixing on multi-channel hardware endpoints"
    )]
    pub(crate) no_surround_upmix: bool,

    #[arg(
        long,
        visible_alias = "mpegh-714",
        default_value_t = false,
        conflicts_with_all = ["no_mpegh", "stereo", "passthrough"],
        help = "Encode 7.1.4 (11.1, CICP Layout 19) output as an MPEG-H 3D Audio IEC 61937-13 HDMI bitstream"
    )]
    pub(crate) mpegh: bool,

    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "mpegh",
        help = "Disable MPEG-H 3D Audio 7.1.4 (11.1) HDMI bitstreaming and use uncompressed LPCM only"
    )]
    pub(crate) no_mpegh: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Headlessly register RustTracker as a Non-Steam shortcut with SteamOS Home Screen artwork and exit (ideal for SSH deployment)"
    )]
    pub(crate) register_steam: bool,
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn env_flag_enabled(val: Option<&str>) -> bool {
    val.is_some_and(|v| {
        let trimmed = v.trim();
        !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
    })
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn should_prefer_x11_backend(
    wayland_env: Option<&str>,
    x11_env: Option<&str>,
    has_wayland_display: bool,
    has_x11_display: bool,
) -> bool {
    let force_x11 = env_flag_enabled(x11_env);
    let disable_wayland = wayland_env.is_some_and(|v| {
        let trimmed = v.trim();
        trimmed == "0" || trimmed.eq_ignore_ascii_case("false")
    });
    let use_wayland = !force_x11 && !disable_wayland && has_wayland_display;
    !use_wayland && (has_x11_display || force_x11)
}

pub(crate) fn has_cli_upmix_override(args: &Args) -> bool {
    args.surround_upmix
        || args.no_surround_upmix
        || args.stereo
        || (args.mpegh && !args.no_surround_upmix)
}

pub(crate) fn apply_cli_audio_flags(state: &mut AppState, args: &Args) {
    if args.stereo {
        state.set_force_stereo_downmix(true);
    }
    if args.passthrough {
        if cfg!(any(target_os = "windows", target_os = "linux")) {
            state.passthrough_enabled = true;
            state.surround_upmix.enabled = false;
            state.force_stereo_downmix = false;
        } else {
            state.passthrough_enabled = false;
            state.osd_text = Some("Bitstream Passthrough requires Linux or Windows".to_string());
            state.osd_timer = 2.5;
        }
    }
    if !args.stereo {
        if args.no_surround_upmix {
            state.set_surround_upmix(false);
        } else if args.surround_upmix {
            state.set_surround_upmix(true);
        }
    }
    if args.no_mpegh {
        state.set_mpegh_bitstream(false);
    } else if args.mpegh {
        if !args.no_surround_upmix && !args.stereo {
            state.set_surround_upmix(true);
        }
        state.set_mpegh_bitstream(true);
    }
    if args.stereo {
        // Ensure combinable flags like `--stereo --no-mpegh` or `--stereo --no-surround-upmix`
        // never overwrite the forced-stereo OSD with LPCM/upmix messages.
        state.osd_text = Some("🎧 Stereo Downmix (2.0): Forced".to_string());
        state.osd_timer = 2.5;
    }
}

/// Updates startup OSD after `audio::start_audio_thread` probes real hardware channels
/// (`apply_cli_audio_flags` runs before stream open while `hardware_channels` is still 2).
pub(crate) fn reconcile_post_load_cli_audio_osd(state: &mut AppState, args: &Args) {
    if args.stereo {
        state.osd_text = Some("🎧 Stereo Downmix (2.0): Forced".to_string());
        state.osd_timer = 2.5;
    } else if args.surround_upmix && !args.mpegh && state.hardware_channels > 2 {
        if state.surround_upmix.enabled {
            let badge = crate::state::SurroundUpmixConfig::layout_badge(state.hardware_channels);
            state.osd_text = Some(format!("🔊 Surround Upmix: ON ({badge})"));
            state.osd_timer = 2.5;
        } else if state.is_native_atmos {
            state.osd_text = Some("🔊 Surround Upmix: Bypassed (Native Dolby Atmos)".to_string());
            state.osd_timer = 2.5;
        } else if !crate::state::SurroundUpmixConfig::can_upmix(
            state.source_channels,
            state.hardware_channels,
        ) {
            state.osd_text = Some(format!(
                "🔊 Surround Upmix: Bypassed (Native {}ch Match)",
                state.source_channels
            ));
            state.osd_timer = 2.5;
        }
    }
}

pub(crate) fn reconcile_passthrough_change_osd(state: &mut AppState, pt_enabled: bool) {
    let existing = state.osd_text.as_deref().unwrap_or_default();
    if state.force_stereo_downmix || existing.contains("Stereo Downmix") {
        if state.force_stereo_downmix {
            state.osd_text = Some("🎧 Stereo Downmix (2.0): Forced".to_string());
        }
        return;
    }
    if state.surround_upmix.enabled && !pt_enabled {
        if !existing.contains("MPEG-H") && !existing.contains("11.1") {
            if state.hardware_channels > 2 {
                let badge =
                    crate::state::SurroundUpmixConfig::layout_badge(state.hardware_channels);
                state.osd_text = Some(format!("🔊 Surround Upmix: ON ({badge})"));
            } else {
                state.osd_text =
                    Some("🔊 Surround Upmix: Armed (Stereo/Mono Output — No Change)".to_string());
            }
        }
    } else if !pt_enabled && (existing.contains("Surround Upmix") || existing.contains("11.1")) {
        // Preserve explicit Surround Upmix OFF or 11.1 MPEG-H OFF OSD across shared-mode reload
    } else {
        state.osd_text = Some(if pt_enabled {
            "Bitstream Passthrough: ON (Exclusive)".to_string()
        } else {
            "Bitstream Passthrough: OFF (Shared Mode)".to_string()
        });
    }
}

pub(crate) fn matches_visualizer_spec(vis: &str) -> Option<usize> {
    let vis_lower = vis.trim().to_lowercase();
    if vis_lower.is_empty() {
        return None;
    }
    let normalize = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let vis_norm = normalize(&vis_lower);
    crate::state::VISUALIZERS.iter().position(|v| {
        let short_name = v
            .filename
            .strip_prefix("vis_")
            .unwrap_or(v.filename)
            .strip_suffix(".wgsl")
            .unwrap_or(v.filename)
            .to_lowercase();
        let name_norm = normalize(v.name);
        let short_norm = normalize(&short_name);

        v.id.to_string() == vis_lower
            || v.name.to_lowercase() == vis_lower
            || short_name == vis_lower
            || (!vis_norm.is_empty() && (name_norm == vis_norm || short_norm == vis_norm))
            || (v.id == 5
                && matches!(
                    vis_norm.as_str(),
                    "retrofire"
                        | "retro_fire"
                        | "flame"
                        | "fire"
                        | "doom"
                        | "doomfire"
                        | "doom_fire"
                        | "psxfire"
                ))
            || (v.id == 6 && matches!(vis_norm.as_str(), "firesim" | "firesimulation"))
            || (v.id == 31
                && matches!(
                    vis_norm.as_str(),
                    "neonroom"
                        | "neonspatialroom"
                        | "acousticwaves"
                        | "acousticwavechamber"
                        | "neonstudio"
                ))
    })
}

pub(crate) fn is_http_or_virtual_uri(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("file://")
        || lower.starts_with("mic://")
}

pub(crate) fn absolutize_ipc_paths(paths: &[String]) -> Vec<String> {
    let cwd = std::env::current_dir().ok();
    paths
        .iter()
        .map(|raw| {
            if is_http_or_virtual_uri(raw) {
                return raw.clone();
            }
            let p = std::path::Path::new(raw);
            if p.is_absolute() {
                raw.clone()
            } else if let Some(ref base) = cwd {
                base.join(p).to_string_lossy().into_owned()
            } else {
                raw.clone()
            }
        })
        .collect()
}

pub(crate) fn looks_like_media_or_path_arg(arg: &std::ffi::OsStr) -> bool {
    let bytes = arg.as_encoded_bytes();
    if bytes == b"--" {
        return true;
    }
    if bytes.is_empty() || bytes[0] == b'-' {
        return false;
    }
    let Some(s) = arg.to_str() else {
        // Non-UTF-8 OS strings cannot be visualizer identifiers; treat as filesystem paths.
        return true;
    };
    if matches!(s.to_ascii_lowercase().as_str(), "list" | "help")
        || matches_visualizer_spec(s).is_some()
    {
        return false;
    }
    if s.contains('/')
        || s.contains('\\')
        || s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("file://")
    {
        return true;
    }
    let path = std::path::Path::new(s);
    if let Some(ext) = path.extension().and_then(|e| e.to_str())
        && (crate::playlist::is_supported_media_extension(ext)
            || crate::playlist::is_playlist_extension(ext))
    {
        return true;
    }
    path.exists()
}

/// Normalizes bare `--bench` and bare `--vis` when immediately followed by `--` or a positional
/// file/URL argument (e.g. `rusttracker --bench song.mod` -> `--bench=300`, `rusttracker --vis song.mod`
/// -> `--vis=`) so `clap` does not swallow the media filename as the optional flag value.
pub(crate) fn normalize_cli_args<I>(raw_args: I) -> Vec<std::ffi::OsString>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let raw: Vec<std::ffi::OsString> = raw_args.into_iter().collect();
    if raw.is_empty() {
        return raw;
    }
    let mut out = Vec::with_capacity(raw.len());
    out.push(raw[0].clone());
    let mut passthrough_positional = false;
    let mut i = 1;
    while i < raw.len() {
        let arg = &raw[i];
        if !passthrough_positional && arg.as_encoded_bytes() == b"--" {
            passthrough_positional = true;
            out.push(arg.clone());
            i += 1;
            continue;
        }
        if !passthrough_positional && arg.as_encoded_bytes() == b"--bench" {
            let next_is_non_numeric_pos = raw.get(i + 1).is_some_and(|n| {
                let bytes = n.as_encoded_bytes();
                if bytes == b"--" {
                    return true;
                }
                !bytes.is_empty()
                    && bytes[0] != b'-'
                    && !bytes.iter().all(|b| b.is_ascii_digit())
                    && n.to_str().is_none_or(|s| s.parse::<u32>().is_err())
            });
            if next_is_non_numeric_pos {
                out.push(std::ffi::OsString::from("--bench=300"));
                i += 1;
                continue;
            }
        }
        if !passthrough_positional
            && arg.as_encoded_bytes() == b"--vis"
            && raw
                .get(i + 1)
                .is_some_and(|n| looks_like_media_or_path_arg(n))
        {
            out.push(std::ffi::OsString::from("--vis="));
            i += 1;
            continue;
        }
        out.push(arg.clone());
        i += 1;
    }
    out
}
