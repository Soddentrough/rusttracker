use crate::state::AppState;
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::{Arc, Mutex};

pub use crate::devices::{
    SYSTEM_DEFAULT_AUDIO_DEVICE, format_cpal_device_name, get_available_audio_devices,
    get_default_audio_device_name, is_default_audio_device_selection, is_relevant_audio_device,
    is_usable_cpal_device_for_mode,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSinkTopology {
    pub sink_id: String,
    pub description: String,
    pub channels: u16,
}

pub fn parse_proc_asound_max_playback_channels(stream_text: &str) -> Option<u16> {
    let mut in_playback = false;
    let mut max_ch: Option<u16> = None;
    for raw_line in stream_text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("Playback:") {
            in_playback = true;
            continue;
        }
        if trimmed.starts_with("Capture:") {
            in_playback = false;
            continue;
        }
        if in_playback
            && let Some(ch_str) = trimmed.strip_prefix("Channels:")
            && let Ok(ch) = ch_str.trim().parse::<u16>()
            && ch > 0
        {
            max_ch = Some(max_ch.map_or(ch, |m| m.max(ch)));
        } else if let Some(ch_str) = trimmed.strip_prefix("channels:")
            && let Ok(ch) = ch_str.trim().parse::<u16>()
            && ch > 0
        {
            max_ch = Some(max_ch.map_or(ch, |m| m.max(ch)));
        }
    }
    max_ch
}

/// Parses an ALSA `/proc/asound/cardN/eld#*` text block, returning `Some((monitor_name, max_lpcm_channels))`
/// when `monitor_present == 1` and `eld_valid == 1`, or `None` when unplugged/invalid.
pub fn parse_alsa_eld_active_info(text: &str) -> Option<(Option<String>, Option<u16>)> {
    let mut present = false;
    let mut valid = false;
    let mut mon: Option<String> = None;
    let mut sad_is_lpcm = [false; 16];
    let mut sad_channels = [0u16; 16];

    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        match key {
            "monitor_present" => present = parts.next() == Some("1"),
            "eld_valid" => valid = parts.next() == Some("1"),
            "monitor_name" => {
                let rest: Vec<&str> = parts.collect();
                if !rest.is_empty() {
                    mon = Some(rest.join(" "));
                }
            }
            _ if key.starts_with("sad") => {
                if let Some(after_sad) = key.strip_prefix("sad") {
                    if let Some(idx_str) = after_sad.strip_suffix("_coding_type")
                        && let Ok(idx) = idx_str.parse::<usize>()
                        && idx < 16
                    {
                        let rest: Vec<&str> = parts.collect();
                        let joined = rest.join(" ");
                        sad_is_lpcm[idx] = joined.contains("[0x1]")
                            || joined.eq_ignore_ascii_case("LPCM")
                            || joined.contains("LPCM");
                    } else if let Some(idx_str) = after_sad.strip_suffix("_channels")
                        && let Ok(idx) = idx_str.parse::<usize>()
                        && idx < 16
                        && let Some(ch_tok) = parts.next()
                        && let Ok(ch) = ch_tok.parse::<u16>()
                    {
                        sad_channels[idx] = ch;
                    }
                }
            }
            _ => {}
        }
    }

    if !present || !valid {
        return None;
    }

    let mut max_lpcm: Option<u16> = None;
    for i in 0..16 {
        if sad_is_lpcm[i] && sad_channels[i] > 0 {
            let ch = sad_channels[i].clamp(1, 32);
            max_lpcm = Some(max_lpcm.map_or(ch, |m| m.max(ch)));
        }
    }
    Some((mon, max_lpcm))
}

/// Parses WirePlumber's persisted state files (`default-nodes` and optional `default-routes`)
/// in-process without spawning `wpctl` or `pactl` child processes, honoring an optional live
/// hardware-presence predicate across ordered fallback candidates (`primary`, `.0`, `.1`, ...).
pub fn parse_wireplumber_state_topology_filtered<F>(
    default_nodes: &str,
    default_routes: Option<&str>,
    mic: bool,
    mut is_candidate_connected: F,
) -> Option<AudioSinkTopology>
where
    F: FnMut(&str) -> bool,
{
    let primary_key = if mic {
        "default.configured.audio.source"
    } else {
        "default.configured.audio.sink"
    };
    let fallback_prefix = if mic {
        "default.configured.audio.source."
    } else {
        "default.configured.audio.sink."
    };

    let mut primary_node: Option<&str> = None;
    let mut numbered_fallbacks: Vec<(u32, &str)> = Vec::new();
    for raw_line in default_nodes.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('[') || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            if k == primary_key {
                primary_node = Some(v);
            } else if let Some(idx_str) = k.strip_prefix(fallback_prefix)
                && let Ok(idx) = idx_str.parse::<u32>()
            {
                numbered_fallbacks.push((idx, v));
            }
        }
    }
    numbered_fallbacks.sort_by_key(|(idx, _)| *idx);

    let mut ordered_candidates: Vec<&str> = Vec::with_capacity(1 + numbered_fallbacks.len());
    if let Some(p) = primary_node {
        ordered_candidates.push(p);
    }
    for (_, fb) in numbered_fallbacks {
        if !ordered_candidates.contains(&fb) {
            ordered_candidates.push(fb);
        }
    }
    if ordered_candidates.is_empty() {
        return None;
    }

    let sink_id = ordered_candidates
        .iter()
        .copied()
        .find(|cand| is_candidate_connected(cand))
        .unwrap_or(ordered_candidates[0])
        .to_string();

    let dir_tag = if mic { "input" } else { "output" };
    let node_prefix = if mic { "alsa_input." } else { "alsa_output." };

    let mut route_channels: Option<u16> = None;
    if let Some(routes_text) = default_routes
        && let Some(rest) = sink_id.strip_prefix(node_prefix)
        && let Some((card_body, profile_suffix)) = rest.rsplit_once('.')
    {
        let card_id = format!("alsa_card.{card_body}");
        let profile_key = format!("{card_id}:profile:{dir_tag}:{profile_suffix}");
        let mut active_route_name: Option<String> = None;

        for raw_line in routes_text.lines() {
            let line = raw_line.trim();
            if let Some((k, v)) = line.split_once('=')
                && k.trim() == profile_key
            {
                let inner = v.trim().trim_start_matches('[').trim_end_matches(']');
                if let Some(first_route) = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"'))
                    .find(|s| !s.is_empty())
                {
                    active_route_name = Some(first_route.to_string());
                    break;
                }
            }
        }

        let target_route_prefix = active_route_name
            .as_deref()
            .map(|r| format!("{card_id}:{dir_tag}:{r}"))
            .unwrap_or_else(|| format!("{card_id}:{dir_tag}:"));

        for raw_line in routes_text.lines() {
            let line = raw_line.trim();
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let k = k.trim();
            let matches_route = if active_route_name.is_some() {
                k == target_route_prefix
            } else {
                k.starts_with(&target_route_prefix)
            };
            if matches_route
                && let Some(cmap_pos) = v.find("\"channelMap\"")
                && let Some(after_key) = v.get(cmap_pos + "\"channelMap\"".len()..)
                && let Some(open_br) = after_key.find('[')
                && let Some(inside) = after_key.get(open_br + 1..)
                && let Some(close_br) = inside.find(']')
            {
                let count = inside[..close_br]
                    .split(',')
                    .map(|s| s.trim().trim_matches('"'))
                    .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("UNKNOWN"))
                    .count() as u16;
                if count > 0 {
                    route_channels = Some(count.clamp(1, 32));
                    break;
                }
            }
        }
    }

    let inferred_channels = {
        let lower = sink_id.to_ascii_lowercase();
        if lower.contains("surround-714") || lower.contains("7.1.4") {
            12
        } else if lower.contains("surround-71") || lower.contains("surround71") {
            8
        } else if lower.contains("surround-51") || lower.contains("surround51") {
            6
        } else if lower.contains("surround-50") || lower.contains("surround-41") {
            5
        } else if lower.contains("surround-40") || lower.contains("surround40") {
            4
        } else if lower.contains("mono") || mic {
            1
        } else {
            2
        }
    };

    let description = humanize_pipewire_node_name(&sink_id);
    Some(AudioSinkTopology {
        sink_id,
        description,
        channels: route_channels.unwrap_or(inferred_channels),
    })
}

/// Parses WirePlumber's persisted state files (`default-nodes` and optional `default-routes`)
/// in-process without spawning `wpctl` or `pactl` child processes.
pub fn parse_wireplumber_state_topology(
    default_nodes: &str,
    default_routes: Option<&str>,
    mic: bool,
) -> Option<AudioSinkTopology> {
    parse_wireplumber_state_topology_filtered(default_nodes, default_routes, mic, |_| true)
}

fn humanize_pipewire_node_name(sink_id: &str) -> String {
    if let Some(rest) = sink_id
        .strip_prefix("alsa_output.")
        .or_else(|| sink_id.strip_prefix("alsa_input."))
    {
        let body = rest.rsplit_once('.').map(|(head, _)| head).unwrap_or(rest);
        if let Some(usb_body) = body.strip_prefix("usb-") {
            let trimmed = usb_body
                .rsplit_once('-')
                .map(|(head, _)| head)
                .unwrap_or(usb_body);
            let cleaned = trimmed.replace('_', " ");
            let collapsed: Vec<&str> = cleaned.split_whitespace().collect();
            if !collapsed.is_empty() {
                return collapsed.join(" ");
            }
        }
    }
    sink_id.to_string()
}

#[cfg(target_os = "linux")]
fn find_alsa_card_index_for_body(card_body: &str) -> Option<u32> {
    if let Some(pci_raw) = card_body.strip_prefix("pci-") {
        let pci_addr = pci_raw.replacen('_', ":", 2);
        if let Ok(entries) = std::fs::read_dir("/sys/class/sound") {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let Some(fname_str) = fname.to_str() else {
                    continue;
                };
                let Some(idx_str) = fname_str.strip_prefix("card") else {
                    continue;
                };
                let Ok(card_num) = idx_str.parse::<u32>() else {
                    continue;
                };
                if let Ok(link_target) = std::fs::read_link(entry.path().join("device"))
                    && link_target
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.eq_ignore_ascii_case(&pci_addr))
                {
                    return Some(card_num);
                }
            }
        }
        return None;
    }

    if card_body.starts_with("usb-") {
        let by_id_path = std::path::Path::new("/dev/snd/by-id").join(card_body);
        if let Ok(link_target) = std::fs::read_link(&by_id_path)
            && let Some(ctrl_name) = link_target.file_name().and_then(|n| n.to_str())
            && let Some(idx_str) = ctrl_name.strip_prefix("controlC")
            && let Ok(card_num) = idx_str.parse::<u32>()
        {
            return Some(card_num);
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn is_linux_alsa_candidate_connected(sink_id: &str, mic: bool) -> bool {
    let Some(rest) = sink_id
        .strip_prefix("alsa_output.")
        .or_else(|| sink_id.strip_prefix("alsa_input."))
    else {
        // Non-ALSA PipeWire nodes (Bluetooth, RAOP, virtual sinks) are accepted unless known absent
        return true;
    };
    if !std::path::Path::new("/sys/class/sound").exists() {
        return true;
    }

    let (card_body, profile_suffix) = rest.rsplit_once('.').unwrap_or((rest, ""));
    if card_body.starts_with("usb-") {
        if std::path::Path::new("/dev/snd/by-id").exists() {
            return std::path::Path::new("/dev/snd/by-id")
                .join(card_body)
                .exists();
        }
        return true;
    }

    if card_body.starts_with("pci-") {
        let Some(card_num) = find_alsa_card_index_for_body(card_body) else {
            return false;
        };
        if !mic && profile_suffix.contains("hdmi") {
            let card_dir = std::path::PathBuf::from(format!("/proc/asound/card{card_num}"));
            let preferred_eld_idx: u32 = profile_suffix
                .rsplit_once("extra")
                .and_then(|(_, n)| n.parse::<u32>().ok())
                .unwrap_or(0);
            let eld_path = card_dir.join(format!("eld#0.{preferred_eld_idx}"));
            if eld_path.exists()
                && let Ok(text) = std::fs::read_to_string(&eld_path)
            {
                return parse_alsa_eld_active_info(&text).is_some();
            }
        }
    }

    true
}

#[cfg(target_os = "linux")]
fn lookup_alsa_card_pretty_name(card_num: u32) -> Option<String> {
    let cards_text = std::fs::read_to_string("/proc/asound/cards").ok()?;
    let prefix = card_num.to_string();
    for line in cards_text.lines() {
        let trimmed = line.trim_start();
        if let Some(after_num) = trimmed.strip_prefix(&prefix)
            && after_num.starts_with(' ')
            && let Some((_, after_sep)) = after_num.split_once(" - ")
        {
            let name = after_sep.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn resolve_alsa_card_enrichment(topo: &mut AudioSinkTopology, mic: bool, has_explicit_route: bool) {
    let Some(rest) = topo
        .sink_id
        .strip_prefix("alsa_output.")
        .or_else(|| topo.sink_id.strip_prefix("alsa_input."))
    else {
        return;
    };
    let (card_body, profile_suffix) = rest.rsplit_once('.').unwrap_or((rest, ""));
    let matched_card_idx = find_alsa_card_index_for_body(card_body);
    let mut card_pretty_name = matched_card_idx.and_then(lookup_alsa_card_pretty_name);

    if let Some(card_num) = matched_card_idx {
        let card_dir = std::path::PathBuf::from(format!("/proc/asound/card{card_num}"));

        // Extract HDMI/DisplayPort ELD monitor/AVR name and LPCM channel capability if active
        if !mic && profile_suffix.contains("hdmi") {
            let preferred_eld_idx: u32 = profile_suffix
                .rsplit_once("extra")
                .and_then(|(_, n)| n.parse::<u32>().ok())
                .unwrap_or(0);

            let mut eld_info =
                std::fs::read_to_string(card_dir.join(format!("eld#0.{preferred_eld_idx}")))
                    .ok()
                    .and_then(|t| parse_alsa_eld_active_info(&t));

            if eld_info.is_none()
                && let Ok(entries) = std::fs::read_dir(&card_dir)
            {
                for entry in entries.flatten() {
                    if entry
                        .file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("eld#"))
                        && let Ok(text) = std::fs::read_to_string(entry.path())
                        && let Some(info) = parse_alsa_eld_active_info(&text)
                    {
                        eld_info = Some(info);
                        break;
                    }
                }
            }

            if let Some((mon_opt, eld_ch_opt)) = eld_info {
                if let Some(mon) = mon_opt {
                    topo.description = match &card_pretty_name {
                        Some(card_name) => format!("{mon} ({card_name})"),
                        None => mon,
                    };
                } else if let Some(card_name) = card_pretty_name.take() {
                    topo.description = card_name;
                }
                if !has_explicit_route
                    && !profile_suffix.contains("stereo")
                    && let Some(eld_ch) = eld_ch_opt
                    && eld_ch > topo.channels
                {
                    topo.channels = eld_ch;
                }
            } else if let Some(card_name) = card_pretty_name.take() {
                topo.description = card_name;
            }
        } else if let Some(card_name) = card_pretty_name {
            topo.description = card_name;
        }

        // If an ALSA playback substream on this card is actively open with > topo.channels, honor it
        if !mic && let Ok(entries) = std::fs::read_dir(&card_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name
                    .to_str()
                    .is_some_and(|n| n.starts_with("pcm") && n.ends_with('p'))
                    && let Ok(hw_params) =
                        std::fs::read_to_string(entry.path().join("sub0/hw_params"))
                    && let Some(open_ch) = parse_proc_asound_max_playback_channels(&hw_params)
                    && open_ch > topo.channels
                {
                    topo.channels = open_ch.clamp(1, 32);
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn query_proc_asound_fallback_topology(mic: bool) -> Option<AudioSinkTopology> {
    let asound_dir = std::path::Path::new("/proc/asound");
    let entries = std::fs::read_dir(asound_dir).ok()?;
    let mut cards: Vec<(u32, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_str()?;
            let idx = s.strip_prefix("card")?.parse::<u32>().ok()?;
            Some((idx, e.path()))
        })
        .collect();
    cards.sort_by_key(|(idx, _)| *idx);

    let pcm_suffix = if mic { 'c' } else { 'p' };
    let node_prefix = if mic { "alsa_input" } else { "alsa_output" };

    // 1. Prefer any card with an actively RUNNING PCM substream
    for &(card_num, ref card_dir) in &cards {
        if let Ok(sub_entries) = std::fs::read_dir(card_dir) {
            for entry in sub_entries.flatten() {
                let Some(pcm_name) = entry.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                if !pcm_name.starts_with("pcm") || !pcm_name.ends_with(pcm_suffix) {
                    continue;
                }
                let sub0 = entry.path().join("sub0");
                if let Ok(status) = std::fs::read_to_string(sub0.join("status"))
                    && status.contains("state: RUNNING")
                {
                    let ch = std::fs::read_to_string(sub0.join("hw_params"))
                        .ok()
                        .and_then(|hp| parse_proc_asound_max_playback_channels(&hp))
                        .unwrap_or(if mic { 1 } else { 2 });
                    let desc = lookup_alsa_card_pretty_name(card_num)
                        .unwrap_or_else(|| format!("ALSA Card {card_num}"));
                    return Some(AudioSinkTopology {
                        sink_id: format!("{node_prefix}.card{card_num}.{pcm_name}"),
                        description: desc,
                        channels: ch,
                    });
                }
            }
        }
    }

    // 2. Otherwise prefer a connected HDMI/DisplayPort ELD sink when monitoring playback
    if !mic {
        for &(card_num, ref card_dir) in &cards {
            if let Ok(sub_entries) = std::fs::read_dir(card_dir) {
                for entry in sub_entries.flatten() {
                    let Some(eld_name) = entry.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    if eld_name.starts_with("eld#")
                        && let Ok(text) = std::fs::read_to_string(entry.path())
                        && let Some((mon_opt, eld_ch_opt)) = parse_alsa_eld_active_info(&text)
                    {
                        let card_name = lookup_alsa_card_pretty_name(card_num);
                        let description = match (mon_opt, card_name) {
                            (Some(mon), Some(cn)) => format!("{mon} ({cn})"),
                            (Some(mon), None) => mon,
                            (None, Some(cn)) => cn,
                            (None, None) => format!("ALSA HDMI Card {card_num}"),
                        };
                        return Some(AudioSinkTopology {
                            sink_id: format!("alsa_output.card{card_num}.{eld_name}"),
                            description,
                            channels: eld_ch_opt.unwrap_or(2),
                        });
                    }
                }
            }
        }
    }

    // 3. Final fallback: first card exposing a matching playback/capture PCM device
    for &(card_num, ref card_dir) in &cards {
        if let Ok(sub_entries) = std::fs::read_dir(card_dir)
            && sub_entries.flatten().any(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("pcm") && n.ends_with(pcm_suffix))
            })
        {
            let desc = lookup_alsa_card_pretty_name(card_num)
                .unwrap_or_else(|| format!("ALSA Card {card_num}"));
            return Some(AudioSinkTopology {
                sink_id: format!("{node_prefix}.card{card_num}"),
                description: desc,
                channels: if mic { 1 } else { 2 },
            });
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn query_linux_audio_sink_topology_in_process(mic: bool) -> Option<AudioSinkTopology> {
    let state_base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state"))
        });

    if let Some(base) = state_base {
        let wp_dir = base.join("wireplumber");
        if let Ok(nodes_text) = std::fs::read_to_string(wp_dir.join("default-nodes")) {
            let routes_text = std::fs::read_to_string(wp_dir.join("default-routes")).ok();
            let has_routes = routes_text.is_some();
            if let Some(mut topo) = parse_wireplumber_state_topology_filtered(
                &nodes_text,
                routes_text.as_deref(),
                mic,
                |cand| is_linux_alsa_candidate_connected(cand, mic),
            ) {
                resolve_alsa_card_enrichment(&mut topo, mic, has_routes);
                return Some(topo);
            }
        }
    }

    // Fallback to PulseAudio persisted default-sink / default-source state file (no subprocess)
    let config_base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    if let Some(cfg_dir) = config_base {
        let suffix = if mic {
            "-default-source"
        } else {
            "-default-sink"
        };
        if let Ok(entries) = std::fs::read_dir(cfg_dir.join("pulse")) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.ends_with(suffix))
                    && let Ok(content) = std::fs::read_to_string(entry.path())
                {
                    let sink_id = content.trim();
                    if !sink_id.is_empty() && is_linux_alsa_candidate_connected(sink_id, mic) {
                        let fake_nodes = if mic {
                            format!("default.configured.audio.source={sink_id}\n")
                        } else {
                            format!("default.configured.audio.sink={sink_id}\n")
                        };
                        if let Some(mut topo) =
                            parse_wireplumber_state_topology(&fake_nodes, None, mic)
                        {
                            resolve_alsa_card_enrichment(&mut topo, mic, false);
                            return Some(topo);
                        }
                    }
                }
            }
        }
    }

    query_proc_asound_fallback_topology(mic)
}

pub fn query_active_audio_sink_topology(mic: bool) -> Option<AudioSinkTopology> {
    #[cfg(target_os = "linux")]
    {
        if let Some(topo) = query_linux_audio_sink_topology_in_process(mic) {
            return Some(topo);
        }
    }

    // On Windows, query the default endpoint's PropertyStore (PKEY_Device_FriendlyName and
    // PKEY_AudioEngine_DeviceFormat) directly without calling `IMMDevice::Activate::<IAudioClient>()`
    // or `GetMixFormat()`, avoiding periodic driver contention and HDMI glitches during playback.
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::PROPERTYKEY;
        use windows::Win32::Media::Audio::{
            IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eCapture, eConsole, eRender,
        };
        use windows::Win32::System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
        };
        use windows::Win32::System::Variant::{VT_BLOB, VT_LPWSTR};
        use windows::core::GUID;

        const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
            fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };
        const PKEY_AUDIO_ENGINE_DEVICE_FORMAT: PROPERTYKEY = PROPERTYKEY {
            fmtid: GUID::from_u128(0xf19f064d_082c_4e27_bc73_6882a1bb8e4c),
            pid: 0,
        };

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            if let Ok(enumerator) =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                && let Ok(dev) = enumerator
                    .GetDefaultAudioEndpoint(if mic { eCapture } else { eRender }, eConsole)
                && let Ok(store) = dev.OpenPropertyStore(STGM_READ)
            {
                let name = store
                    .GetValue(&PKEY_DEVICE_FRIENDLY_NAME)
                    .ok()
                    .and_then(|pv| {
                        if pv.Anonymous.Anonymous.vt == VT_LPWSTR {
                            let pwstr = pv.Anonymous.Anonymous.Anonymous.pwszVal;
                            if !pwstr.is_null() {
                                pwstr.to_string().ok()
                            } else {
                                None
                            }
                        } else {
                            let s = pv.to_string();
                            if s.is_empty() { None } else { Some(s) }
                        }
                    });
                let channels = store
                    .GetValue(&PKEY_AUDIO_ENGINE_DEVICE_FORMAT)
                    .ok()
                    .and_then(|pv| {
                        if pv.Anonymous.Anonymous.vt == VT_BLOB {
                            let blob = pv.Anonymous.Anonymous.Anonymous.blob;
                            if !blob.pBlobData.is_null()
                                && (blob.cbSize as usize) >= std::mem::size_of::<WAVEFORMATEX>()
                            {
                                let wf =
                                    std::ptr::read_unaligned(blob.pBlobData as *const WAVEFORMATEX);
                                Some(wf.nChannels.max(1))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .unwrap_or(if mic { 1 } else { 2 });
                let sink_id = dev
                    .GetId()
                    .ok()
                    .and_then(|s| s.to_string().ok())
                    .or_else(|| name.clone());
                if let (Some(id), Some(desc)) = (sink_id, name) {
                    return Some(AudioSinkTopology {
                        sink_id: id,
                        description: desc,
                        channels,
                    });
                }
            }
        }
    }

    let host = cpal::default_host();
    let default_dev = if mic {
        host.default_input_device()
    } else {
        host.default_output_device()
    }?;
    let desc = default_dev.description().ok()?;
    let name = format_cpal_device_name(&desc);
    let channels = if mic {
        default_dev
            .default_input_config()
            .ok()
            .map(|c| c.channels())
            .unwrap_or(1)
    } else {
        default_dev
            .default_output_config()
            .ok()
            .map(|c| c.channels())
            .unwrap_or(2)
    };
    Some(AudioSinkTopology {
        sink_id: name.clone(),
        description: name,
        channels,
    })
}

pub fn resolve_device_target_channels(
    force_stereo: bool,
    default_device_channels: u16,
    _target_channels: u16,
) -> u16 {
    if force_stereo || default_device_channels <= 2 {
        default_device_channels.clamp(1, 2)
    } else {
        default_device_channels.clamp(1, 12)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyTransitionAction {
    pub reload_stream: bool,
    pub osd_message: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate_topology_transition(
    prev: Option<&AudioSinkTopology>,
    next: &AudioSinkTopology,
    is_auto_device: bool,
    file_loaded: bool,
    is_mic: bool,
    current_hardware_channels: i32,
    force_stereo_downmix: bool,
    is_dummy_audio_output: bool,
    bitstream_active: bool,
    passthrough_enabled: bool,
    os_auto_reroutes_same_channel_streams: bool,
) -> TopologyTransitionAction {
    if !is_auto_device {
        return TopologyTransitionAction {
            reload_stream: false,
            osd_message: None,
        };
    }

    let expected_hw_channels = if force_stereo_downmix {
        (next.channels as i32).clamp(1, 2)
    } else {
        (next.channels as i32).clamp(1, 12)
    };

    let format_sink_osd = |desc: &str, hw_ch: i32| -> String {
        if hw_ch > 2 {
            let badge = crate::state::SurroundUpmixConfig::layout_badge(hw_ch);
            format!("🔊 Audio Output: {} ({})", desc, badge)
        } else {
            let tag = crate::state::SurroundUpmixConfig::short_tag(hw_ch);
            format!("🎧 Audio Output: {} ({})", desc, tag)
        }
    };

    let Some(prev_topo) = prev else {
        let initial_mismatch = (file_loaded || is_mic)
            && (is_dummy_audio_output
                || (current_hardware_channels > 0
                    && !bitstream_active
                    && current_hardware_channels != expected_hw_channels));
        let osd_message = if initial_mismatch {
            Some(format_sink_osd(&next.description, expected_hw_channels))
        } else {
            None
        };
        return TopologyTransitionAction {
            reload_stream: initial_mismatch,
            osd_message,
        };
    };

    if prev_topo.sink_id == next.sink_id
        && prev_topo.channels == next.channels
        && !is_dummy_audio_output
    {
        return TopologyTransitionAction {
            reload_stream: false,
            osd_message: None,
        };
    }

    let channel_mismatch = current_hardware_channels > 0
        && current_hardware_channels != expected_hw_channels
        && !bitstream_active;
    let sink_changed = prev_topo.sink_id != next.sink_id;
    let reload_stream = (file_loaded || is_mic)
        && (is_dummy_audio_output
            || channel_mismatch
            || (sink_changed
                && (!os_auto_reroutes_same_channel_streams
                    || passthrough_enabled
                    || bitstream_active)));

    let osd_message = Some(format_sink_osd(&next.description, expected_hw_channels));

    TopologyTransitionAction {
        reload_stream,
        osd_message,
    }
}

pub fn spawn_audio_topology_watcher(shared_state: Arc<Mutex<AppState>>, is_mic: bool) {
    std::thread::spawn(move || {
        let mut last_topology: Option<AudioSinkTopology> = query_active_audio_sink_topology(is_mic);
        if let Some(ref init_topo) = last_topology
            && let Ok(mut state) = shared_state.lock()
        {
            state.active_audio_sink_id = Some(init_topo.sink_id.clone());
            state.active_audio_sink_description = Some(init_topo.description.clone());
            state.active_audio_sink_channels = init_topo.channels;
        }

        loop {
            std::thread::sleep(std::time::Duration::from_millis(2000));

            let Some(next_topo) = query_active_audio_sink_topology(is_mic) else {
                continue;
            };

            let topology_changed = last_topology.as_ref() != Some(&next_topo);
            if !topology_changed {
                let is_dummy = shared_state
                    .lock()
                    .map(|s| s.is_dummy_audio_output)
                    .unwrap_or(false);
                if !is_dummy {
                    continue;
                }
            }

            let refreshed_devices = if topology_changed {
                Some(get_available_audio_devices(is_mic))
            } else {
                None
            };

            if let Ok(mut state) = shared_state.lock() {
                if let Some(devs) = refreshed_devices {
                    if is_mic {
                        state.available_input_devices = devs.clone();
                    } else {
                        state.available_output_devices = devs.clone();
                    }
                    if state.is_mic_input == is_mic {
                        state.available_audio_devices = devs;
                    }
                }
                let is_auto =
                    is_default_audio_device_selection(state.selected_audio_device.as_deref());
                let exclusive_or_bitstream_active = state.stats.bitstream_active
                    || state.stats.mpegh_bitstream_active
                    || (cfg!(windows)
                        && state.hardware_channels > next_topo.channels as i32
                        && !state.force_stereo_downmix);
                let action = evaluate_topology_transition(
                    last_topology.as_ref(),
                    &next_topo,
                    is_auto,
                    state.file_loaded,
                    is_mic,
                    state.hardware_channels,
                    state.force_stereo_downmix,
                    state.is_dummy_audio_output,
                    exclusive_or_bitstream_active,
                    state.passthrough_enabled || state.surround_upmix.mpegh_bitstream,
                    cfg!(target_os = "linux"),
                );

                state.active_audio_sink_id = Some(next_topo.sink_id.clone());
                state.active_audio_sink_description = Some(next_topo.description.clone());
                state.active_audio_sink_channels = next_topo.channels;

                if action.reload_stream {
                    let msg = action
                        .osd_message
                        .unwrap_or_else(|| format!("Audio Output: {}", next_topo.description));
                    state.audio_topology_reload_request = Some(msg);
                } else if let Some(osd) = action.osd_message {
                    state.osd_text = Some(osd);
                    state.osd_timer = 3.0;
                }
            }

            last_topology = Some(next_topo);
        }
    });
}

pub fn cpal_device_matches_name(desc: &cpal::DeviceDescription, target: &str) -> bool {
    let formatted = format_cpal_device_name(desc);
    formatted == target
        || desc.name() == target
        || formatted.eq_ignore_ascii_case(target)
        || desc.name().eq_ignore_ascii_case(target)
        || desc
            .driver()
            .is_some_and(|drv| drv.eq_ignore_ascii_case(target))
}

pub type OutputConfigRankKey = (
    std::cmp::Reverse<u8>,
    std::cmp::Reverse<bool>,
    std::cmp::Reverse<bool>,
    std::cmp::Reverse<bool>,
    std::cmp::Reverse<u16>,
);

#[allow(clippy::too_many_arguments)]
pub fn rank_output_config_key(
    channels: u16,
    min_rate: u32,
    max_rate: u32,
    sample_format: cpal::SampleFormat,
    target_channels: u16,
    target_rate: u32,
    surround_upmix_enabled: bool,
    force_stereo: bool,
) -> OutputConfigRankKey {
    let ch_rank = if channels == target_channels {
        2u8
    } else if surround_upmix_enabled && !force_stereo && (6..=12).contains(&channels) {
        1u8
    } else {
        0u8
    };
    let stereo = channels >= 2;
    let rate_match = min_rate <= target_rate && max_rate >= target_rate;
    // Prefer IEEE 32-bit float (F32) over 16-bit integer (I16):
    // On Windows WASAPI, cpal maps F32 to WAVE_FORMAT_EXTENSIBLE (KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
    // which is required by Shared Mode for >2ch (5.1/7.1) endpoints and avoids f32->i16->f32 quantization.
    let is_f32 = sample_format == cpal::SampleFormat::F32;

    (
        std::cmp::Reverse(ch_rank),
        std::cmp::Reverse(stereo),
        std::cmp::Reverse(rate_match),
        std::cmp::Reverse(is_f32),
        std::cmp::Reverse(channels),
    )
}
