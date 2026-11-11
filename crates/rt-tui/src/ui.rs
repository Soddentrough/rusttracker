use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph},
};

use crate::state::{AppState, SurroundUpmixConfig, TuiCenterView};

#[derive(Debug, Clone, Copy)]
pub struct TuiLayout {
    pub channels_rect: Rect,
    pub center_rect: Rect,
    pub info_rect: Rect,
    pub spectrum_rect: Rect,
    pub timeline_rect: Rect,
    pub footer_rect: Rect,
    pub device_picker_rect: Option<Rect>,
    pub split_row_y: u16,
}

fn val_to_color(val: f32) -> Color {
    let v = val.clamp(0.0, 100.0);
    if v < 5.0 {
        Color::Rgb(20, 20, 25) // Dark smokey grey
    } else if v < 20.0 {
        Color::Rgb(80, 20, 20) // Deep dark red
    } else if v < 40.0 {
        Color::Rgb(180, 30, 20) // Red
    } else if v < 60.0 {
        Color::Rgb(255, 100, 20) // Orange
    } else if v < 85.0 {
        Color::Rgb(255, 200, 50) // Yellow
    } else {
        Color::Rgb(255, 255, 255) // White Hot
    }
}

/// Approximate terminal column width of a Unicode scalar (2 for wide CJK/emoji, 1 otherwise).
fn char_display_width(c: char) -> usize {
    let u = c as u32;
    if (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0x303E).contains(&u)
        || (0x3040..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE10..=0xFE6F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1FAFF).contains(&u)
        || (0x20000..=0x3FFFD).contains(&u)
    {
        2
    } else {
        1
    }
}

fn str_display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

fn truncate_to_display_width(s: &str, max_cols: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = char_display_width(c);
        if used + w > max_cols {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

fn scroll_text_by_time(text: &str, max_cols: usize, time_secs: f32) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let total_cols = str_display_width(text);
    if total_cols <= max_cols {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let max_scroll = chars.len().saturating_sub(1);
    let scroll_duration = (total_cols.saturating_sub(max_cols) as f32 / 3.0).max(0.5);
    let total_period = 2.0 + scroll_duration + 2.0 + scroll_duration;
    let t = time_secs.max(0.0) % total_period;

    let progress = if t < 2.0 {
        0.0
    } else if t < 2.0 + scroll_duration {
        (t - 2.0) / scroll_duration
    } else if t < 2.0 + scroll_duration + 2.0 {
        1.0
    } else {
        1.0 - ((t - (4.0 + scroll_duration)) / scroll_duration).clamp(0.0, 1.0)
    };

    let extra_cols = total_cols.saturating_sub(max_cols);
    let target_skip_cols = (progress * extra_cols as f32).round() as usize;

    let mut skipped_cols = 0;
    let mut start_idx = 0;
    while start_idx < max_scroll && skipped_cols < target_skip_cols {
        skipped_cols += char_display_width(chars[start_idx]);
        start_idx += 1;
    }

    let mut out = String::new();
    let mut used_cols = 0;
    for &c in &chars[start_idx..] {
        let w = char_display_width(c);
        if used_cols + w > max_cols {
            break;
        }
        used_cols += w;
        out.push(c);
    }
    out
}

fn surround_speaker_labels(num_channels: usize) -> Option<&'static [&'static str]> {
    match num_channels {
        1 => Some(&["M"]),
        2 => Some(&["L", "R"]),
        3 => Some(&["L", "C", "R"]),
        4 => Some(&["Ls", "L", "R", "Rs"]),
        6 => Some(&["Ls", "L", "C", "LFE", "R", "Rs"]),
        8 => Some(&["Lrs", "Ls", "L", "C", "LFE", "R", "Rs", "Rrs"]),
        10 => Some(&["Ltf", "Lrs", "Ls", "L", "C", "LFE", "R", "Rs", "Rrs", "Rtf"]),
        12 => Some(&[
            "Ltr", "Ltf", "Ls", "L", "C", "LFE", "R", "Rs", "Rtf", "Rtr", "Lrs", "Rrs",
        ]),
        _ => None,
    }
}

fn channel_meter_label(state: &AppState, ch_idx: usize, num_ch: usize) -> String {
    let upmix_active = state
        .surround_upmix
        .is_active_for(state.source_channels, state.hardware_channels);
    if state.tracker_channels.is_some() && !upmix_active && num_ch >= 3 {
        if ch_idx == 0 {
            "L".to_string()
        } else if ch_idx + 1 == num_ch {
            "R".to_string()
        } else {
            format!("{}", ch_idx)
        }
    } else if let Some(names) = surround_speaker_labels(num_ch)
        && ch_idx < names.len()
    {
        names[ch_idx].to_string()
    } else {
        format!("{}", ch_idx + 1)
    }
}

/// Centers `text` within a horizontal slot of width `slot_w` so its center aligns with the
/// meter bar located at `[bar_left .. bar_left + bar_w]`.
fn align_text_to_bar(text: &str, slot_w: usize, bar_left: usize, bar_w: usize) -> String {
    if slot_w == 0 {
        return String::new();
    }
    let clipped = truncate_to_display_width(text, slot_w);
    let text_w = str_display_width(&clipped);
    if text_w >= slot_w {
        return format!("{}{}", clipped, " ".repeat(slot_w.saturating_sub(text_w)));
    }
    // Align 2*text_left + text_w with 2*bar_left + bar_w
    let bar_center_x2 = 2 * bar_left + bar_w;
    let ideal_left = bar_center_x2.saturating_sub(text_w) / 2;
    let left_pad = ideal_left.min(slot_w - text_w);
    let right_pad = slot_w - text_w - left_pad;
    format!(
        "{}{}{}",
        " ".repeat(left_pad),
        clipped,
        " ".repeat(right_pad)
    )
}

pub fn build_track_info_lines(state: &AppState, inner_width: usize) -> Vec<Line<'static>> {
    let current_path_str = if state.playlist_index < state.playlist.len() {
        state.playlist[state.playlist_index].clone()
    } else {
        state.song_title.clone()
    };
    let is_mic = crate::state::is_mic_uri(&current_path_str)
        || crate::state::is_mic_uri(&state.song_title)
        || state.is_mic_input;
    let is_network =
        current_path_str.starts_with("http://") || current_path_str.starts_with("https://");

    let scroll_t = if state.is_paused || state.current_seconds <= 0.0 {
        state.tui_uptime_seconds.max(state.current_seconds as f32)
    } else {
        state.current_seconds as f32
    };

    let file_name = if is_mic {
        "Microphone Input".to_string()
    } else if is_network {
        state.song_title.clone()
    } else {
        std::path::Path::new(&state.song_title)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    };

    let file_dir = if is_mic {
        state
            .selected_audio_device
            .clone()
            .unwrap_or_else(|| "Default Input".to_string())
    } else if is_network {
        current_path_str.clone()
    } else {
        let abs_path = if std::path::Path::new(&current_path_str).is_absolute() {
            std::path::PathBuf::from(&current_path_str)
        } else if let Ok(cwd) = std::env::current_dir() {
            cwd.join(&current_path_str)
        } else {
            std::path::PathBuf::from(&current_path_str)
        };
        let path_str = abs_path.to_string_lossy().to_string();
        if let Ok(home) = std::env::var("HOME") {
            if path_str.starts_with(&home) {
                path_str.replacen(&home, "~", 1)
            } else {
                path_str
            }
        } else if let Ok(home) = std::env::var("USERPROFILE") {
            if path_str.starts_with(&home) {
                path_str.replacen(&home, "%USERPROFILE%", 1)
            } else {
                path_str
            }
        } else {
            path_str
        }
    };

    let mut lines = Vec::with_capacity(13);

    // 1. File Name / Title (bold)
    lines.push(Line::from(vec![Span::styled(
        scroll_text_by_time(&file_name, inner_width, scroll_t),
        Style::default().add_modifier(Modifier::BOLD),
    )]));

    // 2. Artist (only if known)
    let trimmed_artist = state.artist.trim();
    if !trimmed_artist.is_empty()
        && !trimmed_artist.eq_ignore_ascii_case("unknown")
        && !trimmed_artist.eq_ignore_ascii_case("unknown artist")
    {
        lines.push(Line::from(vec![
            Span::raw("Artist: "),
            Span::raw(scroll_text_by_time(
                trimmed_artist,
                inner_width.saturating_sub(8),
                scroll_t,
            )),
        ]));
    }

    // 3. Path / URL / Input
    let path_label = if is_mic {
        "Input:  "
    } else if is_network {
        "URL:    "
    } else {
        "Path:   "
    };
    lines.push(Line::from(vec![
        Span::raw(path_label),
        Span::raw(scroll_text_by_time(
            &file_dir,
            inner_width.saturating_sub(8),
            scroll_t,
        )),
    ]));

    // 4. Type
    lines.push(Line::from(vec![
        Span::raw("Type:   "),
        Span::raw(state.module_type.clone()),
    ]));

    // 5. Tracker Tempo / Position (combined onto 1-2 lines so metadata stays compact)
    if state.bpm > 0 || state.speed > 0 {
        let tempo_str = if state.bpm > 0 && state.speed > 0 {
            format!("{} BPM / Spd {}", state.bpm, state.speed)
        } else if state.bpm > 0 {
            format!("{} BPM", state.bpm)
        } else {
            format!("Spd {}", state.speed)
        };
        lines.push(Line::from(vec![
            Span::raw("Tempo:  "),
            Span::raw(tempo_str),
        ]));
    }
    if !state.tracker_patterns_by_order.is_empty() || state.num_patterns > 0 {
        let total_ord = state
            .num_patterns
            .max(state.tracker_patterns_by_order.len() as i32)
            .max(1);
        lines.push(Line::from(vec![
            Span::raw("Pos:    "),
            Span::styled(
                format!(
                    "Ord {:02}/{:02} • Row {:02}",
                    (state.current_tracker_order + 1).max(1),
                    total_ord,
                    state.current_tracker_row.max(0)
                ),
                Style::default().fg(Color::Cyan),
            ),
        ]));
    }

    // 6. Lyrics
    if let Some(lyrics) = &state.lyrics {
        let label = if let Some(fname) = &lyrics.file_name {
            format!("Found ({}) - Synced", fname)
        } else {
            "Found (LRC Synced)".to_string()
        };
        lines.push(Line::from(vec![
            Span::raw("Lyrics: "),
            Span::styled(
                truncate_to_display_width(&label, inner_width.saturating_sub(8)),
                Style::default().fg(Color::Green),
            ),
        ]));
    }

    // 7. Channels & Surround Status
    let upmix_active = state
        .surround_upmix
        .is_active_for(state.source_channels, state.hardware_channels);
    let chan_str = if state.stats.bitstream_active {
        format!(
            "{}ch (Bitstream {})",
            state.num_channels,
            if state.stats.bitstream_format.is_empty() {
                "Passthrough"
            } else {
                &state.stats.bitstream_format
            }
        )
    } else if state.stats.mpegh_bitstream_active {
        format!("{} (7.1.4 MPEG-H 3D)", state.num_channels)
    } else if upmix_active {
        format!(
            "{} ({} Upmix)",
            state.num_channels,
            SurroundUpmixConfig::short_tag(state.hardware_channels)
        )
    } else if let Some(tc) = state.tracker_channels {
        format!("{} ({} trk)", state.num_channels, tc)
    } else {
        state.num_channels.to_string()
    };
    lines.push(Line::from(vec![Span::raw("Chans:  "), Span::raw(chan_str)]));

    // 8. Sample Rate
    lines.push(Line::from(vec![
        Span::raw("Rate:   "),
        Span::raw(format!("{} Hz", state.current_sample_rate as u32)),
    ]));

    // 9. Bitrate
    let effective_bitrate = state.bitrate.or(state.stats.audio_bitrate_kbps);
    lines.push(Line::from(vec![
        Span::raw("Bitr:   "),
        Span::raw(
            effective_bitrate
                .map(|b| format!("{} kbps", b))
                .unwrap_or_else(|| {
                    if is_mic {
                        "Live PCM".to_string()
                    } else if state.tracker_channels.is_some() {
                        "Synth/Tracker".to_string()
                    } else {
                        "Unknown".to_string()
                    }
                }),
        ),
    ]));

    // 10. Length
    let force_hours = state.duration_seconds >= 3600.0 || state.current_seconds >= 3600.0;
    lines.push(Line::from(vec![
        Span::raw("Length: "),
        Span::raw(if state.duration_seconds <= 0.0 {
            "LIVE".to_string()
        } else {
            crate::state::format_timeline_time(state.duration_seconds, force_hours)
        }),
    ]));

    // 11. Audio Track (if multi-track)
    if state.audio_tracks.len() > 1 {
        let num_t = state.audio_tracks.len();
        let mix_key = (num_t + 1).min(9);
        let track_str = if state.active_audio_tracks.len() > 1 {
            let nums: Vec<String> = state
                .active_audio_tracks
                .iter()
                .map(|i| (i + 1).to_string())
                .collect();
            format!(
                "Mix [{}] / {} [1-{} solo]",
                nums.join("+"),
                num_t,
                num_t.min(9)
            )
        } else {
            format!(
                "{}/{} [1-{}, {} mix]",
                state.selected_audio_track + 1,
                num_t,
                num_t.min(9),
                mix_key
            )
        };
        lines.push(Line::from(vec![
            Span::raw("Track:  "),
            Span::styled(
                track_str,
                Style::default().fg(if state.active_audio_tracks.len() > 1 {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]));
    }

    // 12. Next Song (at the bottom, scrollable)
    if state.playlist_index + 1 < state.playlist.len() {
        let next_path = std::path::Path::new(&state.playlist[state.playlist_index + 1]);
        let next_song = next_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        lines.push(Line::from(vec![
            Span::styled("Next:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                scroll_text_by_time(&next_song, inner_width.saturating_sub(8), scroll_t),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    lines
}

/// Computes the exact layout rectangles for all TUI panels and modals.
/// Shared by `draw()` and `run_tui()` mouse hit-testing.
pub fn compute_tui_layout(area: Rect, state: &AppState, meta_line_count: usize) -> TuiLayout {
    let usable_h = area.height.saturating_sub(2); // margin(1) top + bottom
    let content_h = usable_h.saturating_sub(2); // timeline (1) + footer (1)
    let needed_top_h = (meta_line_count as u16).saturating_add(2).max(12);

    // At default split_ratio (0.5), size the top row to fit all Track Info lines (at least 14 rows),
    // while allowing `[` and `]` (which adjust panel_split_ratio in 0.15..=0.85) to resize the split.
    let base_top_h = needed_top_h.max(14);
    let delta_ratio = state.panel_split_ratio - 0.5;
    let raw_top_h = (base_top_h as f32 + delta_ratio * (content_h as f32)).round() as i32;
    let min_spec_h = 6i32;
    let max_top_h = (content_h as i32 - min_spec_h).max(6);
    let top_h = raw_top_h.clamp(6, max_top_h) as u16;

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Min(6),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let width = area.width;
    let top_constraints = if width < 80 {
        vec![
            Constraint::Percentage(40),
            Constraint::Percentage(0),
            Constraint::Percentage(60),
        ]
    } else if width < 110 {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ]
    } else {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(45),
            Constraint::Percentage(25),
        ]
    };

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(top_constraints)
        .split(main_chunks[0]);

    let device_picker_rect = if state.show_tui_device_picker {
        let modal_width = 56.min(area.width.saturating_sub(4)).max(10);
        let modal_height = (state.available_audio_devices.len() as u16 + 2)
            .max(4)
            .min(area.height.saturating_sub(4).max(4));
        Some(Rect::new(
            area.x + (area.width.saturating_sub(modal_width)) / 2,
            area.y + (area.height.saturating_sub(modal_height)) / 2,
            modal_width,
            modal_height,
        ))
    } else {
        None
    };

    TuiLayout {
        channels_rect: top_chunks[0],
        center_rect: top_chunks[1],
        info_rect: top_chunks[2],
        spectrum_rect: main_chunks[1],
        timeline_rect: main_chunks[2],
        footer_rect: main_chunks[3],
        device_picker_rect,
        split_row_y: main_chunks[0].bottom(),
    }
}

pub fn draw(f: &mut Frame, state: &AppState) {
    let area = f.area();
    let est_info_w = if area.width < 80 {
        (area.width as usize * 60) / 100
    } else if area.width < 110 {
        (area.width as usize * 30) / 100
    } else {
        (area.width as usize * 25) / 100
    }
    .saturating_sub(4)
    .max(10);

    let pre_meta_lines = build_track_info_lines(state, est_info_w);
    let layout = compute_tui_layout(area, state, pre_meta_lines.len());

    // 1. Layered Channel VUs with Peak Decay, Multi-Column Bars, and dB Scale
    let vu_height = layout.channels_rect.height.saturating_sub(2) as usize;
    let vu_width = layout.channels_rect.width.saturating_sub(2) as usize;
    let num_ch = state.channel_vus.len();

    let mut vu_lines = Vec::new();
    if num_ch == 0 || vu_width == 0 || vu_height == 0 {
        if vu_height > 0 {
            let mid = vu_height / 2;
            for r in 0..vu_height {
                if r == mid {
                    let msg = truncate_to_display_width("No active channels", vu_width);
                    let pad = vu_width.saturating_sub(msg.len()) / 2;
                    vu_lines.push(Line::from(vec![Span::styled(
                        format!("{}{}", " ".repeat(pad), msg),
                        Style::default().fg(Color::DarkGray),
                    )]));
                } else {
                    vu_lines.push(Line::raw(""));
                }
            }
        }
    } else {
        // Reserve a 5-column dBFS ruler on the left when there is room for wide channel slots
        let show_db_scale =
            vu_width >= num_ch.saturating_mul(3).saturating_add(5) && vu_height >= 5;
        let scale_w = if show_db_scale { 5 } else { 0 };
        let plot_w = vu_width.saturating_sub(scale_w).max(1);
        let num_cols = num_ch.min(plot_w.max(1));
        let avg_slot_w = plot_w / num_cols;
        let show_peak_readout = vu_height >= 7 && avg_slot_w >= 4;
        let bar_rows = vu_height
            .saturating_sub(1 + usize::from(show_peak_readout))
            .max(1);
        let denom = bar_rows.max(1) as f32;

        // Precompute per-column slot geometry, aggregated VU, and peak level
        struct ColSpec {
            start_ch: usize,
            slot_w: usize,
            bar_w: usize,
            left_pad: usize,
            right_pad: usize,
            vu: f32,
            peak: f32,
        }
        let mut cols = Vec::with_capacity(num_cols);
        for col in 0..num_cols {
            let x0 = (col * plot_w) / num_cols;
            let x1 = ((col + 1) * plot_w) / num_cols;
            let slot_w = (x1 - x0).max(1);
            let bar_w = match slot_w {
                1 => 1,
                2 => 1,
                3 => 2,
                4 => 3,
                _ => ((slot_w * 2) / 3)
                    .clamp(3, slot_w.saturating_sub(2))
                    .min(12),
            };
            let left_pad = (slot_w - bar_w) / 2;
            let right_pad = slot_w - bar_w - left_pad;

            let start_ch = (col * num_ch) / num_cols;
            let end_ch = (((col + 1) * num_ch) / num_cols)
                .max(start_ch + 1)
                .min(num_ch);
            let mut vu = 0.0f32;
            let mut peak = 0.0f32;
            for ch in start_ch..end_ch {
                vu = vu.max(state.channel_vus[ch]);
                if ch < state.peak_vus.len() {
                    peak = peak.max(state.peak_vus[ch]);
                }
            }
            cols.push(ColSpec {
                start_ch,
                slot_w,
                bar_w,
                left_pad,
                right_pad,
                vu,
                peak,
            });
        }

        // Optional top row: live peak dBFS readout per channel
        if show_peak_readout {
            let mut db_spans = Vec::with_capacity(num_cols + 1);
            if show_db_scale {
                db_spans.push(Span::styled(" dBFS", Style::default().fg(Color::DarkGray)));
            }
            for c in &cols {
                let (txt, style) = if c.peak <= 0.01 {
                    (
                        " -∞ ".to_string(),
                        Style::default().fg(Color::Rgb(75, 75, 82)),
                    )
                } else {
                    let db = (c.peak.clamp(0.0, 1.0) * 60.0 - 60.0).min(0.0);
                    let formatted = if c.slot_w >= 6 && db > -10.0 {
                        format!("{:>4.1}", db)
                    } else {
                        format!("{:>3.0}", db.round())
                    };
                    let fg = if db >= -1.5 {
                        Color::LightRed
                    } else if db >= -6.0 {
                        Color::Yellow
                    } else {
                        Color::Rgb(165, 165, 175)
                    };
                    (formatted, Style::default().fg(fg))
                };
                db_spans.push(Span::styled(
                    align_text_to_bar(&txt, c.slot_w, c.left_pad, c.bar_w),
                    style,
                ));
            }
            vu_lines.push(Line::from(db_spans));
        }

        const SUB_BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

        // Render VUs top to bottom, leaving 1 line at the bottom for labels
        for row in (0..bar_rows).rev() {
            let mut spans = Vec::with_capacity(num_cols * 2 + 1);
            let threshold = (row as f32) / denom;
            let next_threshold = ((row + 1) as f32) / denom;
            let is_top_row = row + 1 >= bar_rows;

            if show_db_scale {
                let scale_label = if is_top_row {
                    "   0┤".to_string()
                } else if row == 0 {
                    " -60┤".to_string()
                } else if bar_rows >= 6 && row == bar_rows / 2 {
                    " -30┤".to_string()
                } else if bar_rows >= 9 && row == (bar_rows * 3) / 4 {
                    " -15┤".to_string()
                } else if bar_rows >= 9 && row == bar_rows / 4 {
                    " -45┤".to_string()
                } else {
                    "    │".to_string()
                };
                spans.push(Span::styled(
                    scale_label,
                    Style::default().fg(Color::Rgb(90, 90, 100)),
                ));
            }

            for c in &cols {
                let is_peak_cap =
                    c.peak > 0.01 && c.peak >= threshold && (c.peak < next_threshold || is_top_row);
                let (bar_str, style) = if is_peak_cap {
                    ("▄".repeat(c.bar_w), Style::default().fg(Color::White))
                } else if c.vu >= next_threshold {
                    (
                        "█".repeat(c.bar_w),
                        Style::default().fg(val_to_color(c.vu * 100.0)),
                    )
                } else if c.vu > threshold
                    && c.vu > 0.01
                    && (c.peak <= next_threshold || c.peak <= 0.01)
                {
                    let frac = ((c.vu - threshold) / (next_threshold - threshold).max(1e-5))
                        .clamp(0.0, 1.0);
                    let idx = ((frac * 8.0).round() as usize).clamp(1, 8) - 1;
                    (
                        SUB_BLOCKS[idx].repeat(c.bar_w),
                        Style::default().fg(val_to_color(c.vu * 100.0)),
                    )
                } else if c.peak > threshold && c.peak > 0.01 {
                    let diff = (c.peak - c.vu).max(0.001);
                    let dist = (threshold - c.vu).max(0.0);
                    let fade_ratio = (dist / diff).clamp(0.0, 1.0);
                    (
                        "█".repeat(c.bar_w),
                        Style::default().fg(val_to_color(100.0 - (fade_ratio * 100.0))),
                    )
                } else if c.slot_w >= 3 {
                    (
                        "░".repeat(c.bar_w),
                        Style::default().fg(Color::Rgb(28, 28, 34)),
                    )
                } else {
                    (" ".repeat(c.bar_w), Style::default())
                };

                if c.left_pad > 0 {
                    spans.push(Span::raw(" ".repeat(c.left_pad)));
                }
                spans.push(Span::styled(bar_str, style));
                if c.right_pad > 0 {
                    spans.push(Span::raw(" ".repeat(c.right_pad)));
                }
            }
            vu_lines.push(Line::from(spans));
        }

        // Bottom row: Channel / Speaker labels centered directly beneath each bar
        let mut vu_label_spans = Vec::with_capacity(num_cols + 1);
        if show_db_scale {
            vu_label_spans.push(Span::styled("  Ch ", Style::default().fg(Color::DarkGray)));
        }
        for (col_idx, c) in cols.iter().enumerate() {
            let label = if num_cols == num_ch {
                channel_meter_label(state, c.start_ch, num_ch)
            } else {
                format!("{}", c.start_ch + 1)
            };
            let show = c.slot_w >= 2 || (col_idx % 2 == 0) || c.slot_w == 1;

            let padded_label = if !show {
                " ".repeat(c.slot_w)
            } else if c.slot_w == 1 {
                if label.len() == 1 {
                    label.clone()
                } else {
                    let n = c.start_ch + 1;
                    if n <= 9 {
                        format!("{}", n)
                    } else if n <= 35 {
                        ((b'A' + (n as u8 - 10)) as char).to_string()
                    } else {
                        "+".to_string()
                    }
                }
            } else {
                align_text_to_bar(&label, c.slot_w, c.left_pad, c.bar_w)
            };
            let is_master_lr = state.tracker_channels.is_some()
                && !state
                    .surround_upmix
                    .is_active_for(state.source_channels, state.hardware_channels)
                && num_ch >= 3
                && (c.start_ch == 0 || c.start_ch + 1 == num_ch);
            let lbl_color = if is_master_lr {
                Color::Cyan
            } else {
                Color::Gray
            };
            vu_label_spans.push(Span::styled(padded_label, Style::default().fg(lbl_color)));
        }
        vu_lines.push(Line::from(vu_label_spans));
    }

    let channels_title = if state
        .surround_upmix
        .is_active_for(state.source_channels, state.hardware_channels)
    {
        format!(
            "Channels ({} Upmix)",
            SurroundUpmixConfig::short_tag(state.hardware_channels)
        )
    } else if let Some(tc) = state.tracker_channels {
        format!("Channels ({} Trk + L/R)", tc)
    } else if num_ch > 0 {
        format!(
            "Channels ({})",
            SurroundUpmixConfig::short_tag(num_ch as i32)
        )
    } else {
        "Channels".to_string()
    };

    let vu_paragraph = Paragraph::new(vu_lines)
        .block(Block::default().title(channels_title).borders(Borders::ALL));
    f.render_widget(vu_paragraph, layout.channels_rect);

    // 2. Top-Center Panel: Lyrics / Tracker Pattern / Heatmap History / Surround EQ
    if layout.center_rect.width > 2 && layout.center_rect.height > 2 {
        let center_width = layout.center_rect.width.saturating_sub(2) as usize;
        let center_height = layout.center_rect.height.saturating_sub(2) as usize;
        let active_view = state.effective_tui_center_view();

        match active_view {
            TuiCenterView::Lyrics => {
                if let Some(lyrics) = &state.lyrics {
                    let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
                    let active_idx = lyrics.find_current_line_idx(display_secs);
                    let is_intro = active_idx.is_none();
                    let base_idx = active_idx.unwrap_or(0);
                    let center_row = center_height / 2;

                    let mut lyric_lines = Vec::with_capacity(center_height);
                    for r in 0..center_height {
                        let offset = (r as i32) - (center_row as i32);
                        let target_idx =
                            (base_idx as i32) + offset - if is_intro && offset > 0 { 1 } else { 0 };

                        if is_intro && offset == 0 {
                            let intro_text = "♪ ♪ ♪  (Intro)";
                            let w = str_display_width(intro_text);
                            let padding = center_width.saturating_sub(w) / 2;
                            let padded = format!("{}{}", " ".repeat(padding), intro_text);
                            lyric_lines.push(Line::from(vec![Span::styled(
                                padded,
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            )]));
                        } else if is_intro && offset < 0 {
                            lyric_lines.push(Line::raw(""));
                        } else if target_idx >= 0 && (target_idx as usize) < lyrics.lines.len() {
                            let line = &lyrics.lines[target_idx as usize];
                            let text = if line.text.is_empty() {
                                "♪ ♪ ♪"
                            } else {
                                &line.text
                            };
                            let truncated =
                                truncate_to_display_width(text, center_width.saturating_sub(4));
                            let trunc_w = str_display_width(&truncated);
                            let padding = center_width.saturating_sub(trunc_w + 4) / 2;

                            if !is_intro && offset == 0 {
                                let formatted = format!("{}> {} <", " ".repeat(padding), truncated);
                                lyric_lines.push(Line::from(vec![Span::styled(
                                    formatted,
                                    Style::default()
                                        .fg(Color::Yellow)
                                        .add_modifier(Modifier::BOLD),
                                )]));
                            } else {
                                let distance = offset.abs();
                                let fg_color = if distance == 1 {
                                    Color::Rgb(200, 200, 200)
                                } else if distance == 2 {
                                    Color::Rgb(140, 140, 140)
                                } else {
                                    Color::Rgb(80, 80, 80)
                                };
                                let formatted = format!("{}{}", " ".repeat(padding + 2), truncated);
                                lyric_lines.push(Line::from(vec![Span::styled(
                                    formatted,
                                    Style::default().fg(fg_color),
                                )]));
                            }
                        } else {
                            lyric_lines.push(Line::raw(""));
                        }
                    }

                    let lyrics_paragraph = Paragraph::new(lyric_lines).block(
                        Block::default()
                            .title("Lyrics [v/Tab: View]")
                            .borders(Borders::ALL),
                    );
                    f.render_widget(lyrics_paragraph, layout.center_rect);
                }
            }
            TuiCenterView::Tracker => {
                let current_order = state.current_tracker_order;
                let current_row = state.current_tracker_row;
                let center_row = (center_height / 2) as i32;
                let max_text_cols = center_width.saturating_sub(6);

                let mut pattern_lines = Vec::with_capacity(center_height);
                for r in 0..center_height {
                    let offset = (r as i32) - center_row;
                    let mut resolved_order = current_order;
                    let mut resolved_row = current_row + offset;

                    if offset < 0 {
                        let history_idx = (-offset - 1) as usize;
                        if history_idx < state.tracker_row_history.len() {
                            let (hist_order, hist_row) = state.tracker_row_history[history_idx];
                            resolved_order = hist_order;
                            resolved_row = hist_row;
                        } else {
                            while resolved_row < 0 && resolved_order > 0 {
                                resolved_order -= 1;
                                if (resolved_order as usize) < state.tracker_patterns_by_order.len()
                                {
                                    resolved_row +=
                                        state.tracker_patterns_by_order[resolved_order as usize]
                                            .len() as i32;
                                } else {
                                    break;
                                }
                            }
                        }
                    } else if offset > 0 {
                        while (resolved_order as usize) < state.tracker_patterns_by_order.len() {
                            let pat_len = state.tracker_patterns_by_order[resolved_order as usize]
                                .len() as i32;
                            if pat_len <= 0 {
                                break;
                            }
                            if resolved_row >= pat_len {
                                resolved_row -= pat_len;
                                resolved_order += 1;
                            } else {
                                break;
                            }
                        }
                    }

                    if resolved_order >= 0
                        && (resolved_order as usize) < state.tracker_patterns_by_order.len()
                    {
                        let pattern = &state.tracker_patterns_by_order[resolved_order as usize];
                        if resolved_row >= 0 && (resolved_row as usize) < pattern.len() {
                            let row_str = &pattern[resolved_row as usize];
                            let slice = truncate_to_display_width(row_str, max_text_cols);

                            if offset == 0 {
                                let formatted = format!("▶{:02} │ {}", resolved_row, slice);
                                let pad =
                                    center_width.saturating_sub(str_display_width(&formatted));
                                pattern_lines.push(Line::from(vec![Span::styled(
                                    format!("{}{}", formatted, " ".repeat(pad)),
                                    Style::default()
                                        .fg(Color::Yellow)
                                        .bg(Color::Rgb(30, 42, 62))
                                        .add_modifier(Modifier::BOLD),
                                )]));
                            } else {
                                let dist = offset.abs();
                                let fg_col = if resolved_row == 0 {
                                    Color::Cyan
                                } else if dist == 1 {
                                    Color::Rgb(205, 210, 220)
                                } else if dist <= 3 {
                                    Color::Rgb(145, 150, 160)
                                } else {
                                    Color::Rgb(85, 90, 100)
                                };
                                let sep = if resolved_row == 0 { "┼" } else { "│" };
                                pattern_lines.push(Line::from(vec![Span::styled(
                                    format!(" {:02} {} {}", resolved_row, sep, slice),
                                    Style::default().fg(fg_col),
                                )]));
                            }
                        } else {
                            pattern_lines.push(Line::raw(""));
                        }
                    } else if offset == 0 && !state.current_tracker_row_string.is_empty() {
                        let slice = truncate_to_display_width(
                            &state.current_tracker_row_string,
                            max_text_cols,
                        );
                        pattern_lines.push(Line::from(vec![Span::styled(
                            format!("▶{:02} │ {}", current_row.max(0), slice),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )]));
                    } else {
                        pattern_lines.push(Line::raw(""));
                    }
                }

                let total_ord = state
                    .num_patterns
                    .max(state.tracker_patterns_by_order.len() as i32)
                    .max(1);
                let title = format!(
                    "Tracker Pattern (Ord {:02}/{:02} • Row {:02}) [v/Tab]",
                    (current_order + 1).max(1),
                    total_ord,
                    current_row.max(0)
                );
                let tracker_paragraph = Paragraph::new(pattern_lines)
                    .block(Block::default().title(title).borders(Borders::ALL));
                f.render_widget(tracker_paragraph, layout.center_rect);
            }
            TuiCenterView::SurroundEq => {
                let cfg = &state.surround_upmix;
                let upmix_active =
                    cfg.is_active_for(state.source_channels, state.hardware_channels);
                let badge = SurroundUpmixConfig::layout_badge(state.hardware_channels);
                let mut eq_lines = Vec::with_capacity(center_height);

                let status_text = if state.stats.mpegh_bitstream_active {
                    format!("ON ({badge} • MPEG-H 3D)")
                } else if upmix_active {
                    format!("ON ({badge})")
                } else if cfg.enabled {
                    format!("ARMED ({badge})")
                } else {
                    "OFF (Bypassed)".to_string()
                };
                let status_col = if upmix_active || state.stats.mpegh_bitstream_active {
                    Color::Green
                } else if cfg.enabled {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };

                eq_lines.push(Line::from(vec![
                    Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        truncate_to_display_width(&status_text, center_width.saturating_sub(8)),
                        Style::default().fg(status_col).add_modifier(Modifier::BOLD),
                    ),
                ]));

                let xo_line = format!(
                    "Sub XO: {:.0}Hz │ Height XO: {:.0}Hz │ Width: {:.0}%",
                    cfg.sub_crossover_hz,
                    cfg.height_crossover_hz,
                    cfg.stereo_width * 100.0
                );
                eq_lines.push(Line::from(vec![Span::styled(
                    truncate_to_display_width(&xo_line, center_width),
                    Style::default().fg(Color::Cyan),
                )]));

                let bar_w = center_width.saturating_sub(18).clamp(6, 28);
                for (i, label) in SurroundUpmixConfig::EQ_BAND_LABELS.iter().enumerate() {
                    if eq_lines.len() + 1 >= center_height {
                        break;
                    }
                    let gain = cfg.band_gains_db[i].clamp(-12.0, 12.0);
                    let norm = ((gain + 12.0) / 24.0).clamp(0.0, 1.0);
                    let filled = (norm * bar_w as f32).round() as usize;
                    let bar_str = format!(
                        "{}{}",
                        "█".repeat(filled),
                        "░".repeat(bar_w.saturating_sub(filled))
                    );
                    let col = if gain.abs() < 0.05 {
                        Color::Gray
                    } else if gain > 0.0 {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    };
                    eq_lines.push(Line::from(vec![
                        Span::raw(format!("{:<6} ", label)),
                        Span::styled(bar_str, Style::default().fg(col)),
                        Span::raw(format!(" {:+5.1}dB", gain)),
                    ]));
                }

                let spk_levels = [
                    ("Front", cfg.front_level, Color::Green),
                    ("Center", cfg.center_level, Color::Cyan),
                    ("Surr", cfg.side_rear_level, Color::Yellow),
                    ("Height", cfg.height_level, Color::Magenta),
                    ("LFE", cfg.lfe_level, Color::Red),
                ];
                for (name, lvl, col) in spk_levels {
                    if eq_lines.len() + 1 >= center_height {
                        break;
                    }
                    let norm = (lvl / 1.5).clamp(0.0, 1.0);
                    let filled = (norm * bar_w as f32).round() as usize;
                    let bar_str = format!(
                        "{}{}",
                        "▰".repeat(filled),
                        "▱".repeat(bar_w.saturating_sub(filled))
                    );
                    eq_lines.push(Line::from(vec![
                        Span::raw(format!("{:<6} ", name)),
                        Span::styled(bar_str, Style::default().fg(col)),
                        Span::raw(format!(" {:>4.0}%", lvl * 100.0)),
                    ]));
                }

                while eq_lines.len() + 1 < center_height {
                    eq_lines.push(Line::raw(""));
                }
                if eq_lines.len() < center_height {
                    let hint = "[u] Upmix  [m] 7.1.4 MPEG-H  [r] Reset  [v/Tab] View";
                    eq_lines.push(Line::from(vec![Span::styled(
                        truncate_to_display_width(hint, center_width),
                        Style::default().fg(Color::DarkGray),
                    )]));
                }

                let eq_paragraph = Paragraph::new(eq_lines).block(
                    Block::default()
                        .title("Surround Upmix & EQ [v/Tab: View]")
                        .borders(Borders::ALL),
                );
                f.render_widget(eq_paragraph, layout.center_rect);
            }
            TuiCenterView::Heatmap => {
                let num_cols = center_width.max(1);
                let mut heatmap_lines = Vec::with_capacity(center_height);
                let history_len = state.spectrum_history.len();
                let total_history_lines = history_len / 2;
                let start_line = total_history_lines.saturating_sub(center_height);

                for cell_y in start_line..total_history_lines {
                    let mut spans = Vec::new();
                    let top_row_idx = cell_y * 2;
                    let bottom_row_idx = cell_y * 2 + 1;

                    if top_row_idx < history_len && bottom_row_idx < history_len {
                        let top_row = &state.spectrum_history[top_row_idx];
                        let bottom_row = &state.spectrum_history[bottom_row_idx];
                        let row_len = top_row.len().min(bottom_row.len());

                        if row_len > 0 {
                            let mut run_fg = Color::Reset;
                            let mut run_bg = Color::Reset;
                            let mut run_len = 0usize;

                            for col in 0..num_cols {
                                let start_bin = (col * row_len) / num_cols;
                                let end_bin = (((col + 1) * row_len) / num_cols)
                                    .max(start_bin + 1)
                                    .min(row_len);
                                let mut top_val = 0.0f32;
                                let mut bot_val = 0.0f32;
                                for b in start_bin..end_bin {
                                    top_val = top_val.max(top_row[b]);
                                    bot_val = bot_val.max(bottom_row[b]);
                                }
                                let fg_col = val_to_color(top_val);
                                let bg_col = val_to_color(bot_val);

                                if run_len > 0 && fg_col == run_fg && bg_col == run_bg {
                                    run_len += 1;
                                } else {
                                    if run_len > 0 {
                                        spans.push(Span::styled(
                                            "▀".repeat(run_len),
                                            Style::default().fg(run_fg).bg(run_bg),
                                        ));
                                    }
                                    run_fg = fg_col;
                                    run_bg = bg_col;
                                    run_len = 1;
                                }
                            }
                            if run_len > 0 {
                                spans.push(Span::styled(
                                    "▀".repeat(run_len),
                                    Style::default().fg(run_fg).bg(run_bg),
                                ));
                            }
                        }
                    }
                    heatmap_lines.push(Line::from(spans));
                }

                let heatmap_paragraph = Paragraph::new(heatmap_lines).block(
                    Block::default()
                        .title("Heatmap History [v/Tab: View]")
                        .borders(Borders::ALL),
                );
                f.render_widget(heatmap_paragraph, layout.center_rect);
            }
        }
    }

    // 3. Metadata / Track Info
    let info_inner_w = layout.info_rect.width.saturating_sub(2) as usize;
    let info_inner_h = layout.info_rect.height.saturating_sub(2) as usize;
    let mut meta_lines = build_track_info_lines(state, info_inner_w);
    if meta_lines.len() > info_inner_h && info_inner_h >= 2 {
        // Keep the title (row 0) and the bottom-most status rows when user shrinks the top panel
        let tail_count = info_inner_h - 1;
        let tail_start = meta_lines.len() - tail_count;
        let mut compacted = Vec::with_capacity(info_inner_h);
        compacted.push(meta_lines[0].clone());
        compacted.extend_from_slice(&meta_lines[tail_start..]);
        meta_lines = compacted;
    }

    let title_text = if state.playlist.len() > 1 {
        let clamped_idx = (state.playlist_index + 1).min(state.playlist.len());
        format!("Track Info ({}/{})", clamped_idx, state.playlist.len())
    } else {
        "Track Info".to_string()
    };

    let meta_paragraph =
        Paragraph::new(meta_lines).block(Block::default().title(title_text).borders(Borders::ALL));
    f.render_widget(meta_paragraph, layout.info_rect);

    // 4. Spectrum Analyzer
    let spec_height = layout.spectrum_rect.height.saturating_sub(2) as usize;
    let spec_width = layout.spectrum_rect.width.saturating_sub(2) as usize;
    let spec_len = state.spectrum_data.len();
    let num_cols = spec_width.clamp(1, spec_len.max(1));
    let chars_per_spec = (spec_width / num_cols).max(1);
    let mut spec_lines = Vec::with_capacity(spec_height);
    let bar_rows = spec_height.saturating_sub(1);
    let denom = bar_rows.max(1) as f32;

    for row in (0..bar_rows).rev() {
        let mut spans = Vec::with_capacity(num_cols);
        let threshold = (row as f32) / denom * 100.0;
        let next_threshold = ((row + 1) as f32) / denom * 100.0;
        let is_top_row = row + 1 >= bar_rows;

        for col in 0..num_cols {
            let start_bin = (col * spec_len) / num_cols;
            let end_bin = (((col + 1) * spec_len) / num_cols)
                .max(start_bin + 1)
                .min(spec_len);
            let mut val = 0.0f32;
            let mut peak = 0.0f32;
            for b in start_bin..end_bin {
                val = val.max(state.spectrum_data[b]);
                if b < state.spectrum_peaks.len() {
                    peak = peak.max(state.spectrum_peaks[b]);
                }
            }

            let is_peak_cap =
                peak > 0.5 && peak >= threshold && (peak < next_threshold || is_top_row);
            let symbol = if is_peak_cap {
                "▄"
            } else if val >= threshold || (peak > threshold && peak > 0.5) {
                "█"
            } else {
                " "
            };

            let padded_symbol = match chars_per_spec {
                1 => symbol.to_string(),
                2 => format!("{} ", symbol),
                _ => format!(
                    " {}{}",
                    symbol,
                    " ".repeat(chars_per_spec.saturating_sub(2))
                ),
            };

            let mut style = Style::default();
            if is_peak_cap {
                style = Style::default().fg(Color::White); // Peak cap
            } else if peak > threshold && val < threshold && peak > 0.5 {
                let diff = (peak - val).max(0.001);
                let dist = threshold - val;
                let fade_ratio = (dist / diff).clamp(0.0, 1.0);
                style = Style::default().fg(val_to_color(100.0 - (fade_ratio * 100.0)));
            } else if val >= threshold && val > 0.5 {
                style = Style::default().fg(val_to_color(val));
            }

            spans.push(Span::styled(padded_symbol, style));
        }
        spec_lines.push(Line::from(spans));
    }

    // Spectrum Labels (Logarithmic 20Hz..max_frequency mapped to terminal columns)
    let mut label_line = vec![' '; spec_width];
    let mut write_label = |col: usize, text: &str| {
        if spec_width == 0 {
            return;
        }
        let mut start_idx = col * chars_per_spec;
        if start_idx + text.len() > spec_width {
            start_idx = spec_width.saturating_sub(text.len());
        }
        for (j, c) in text.chars().enumerate() {
            if start_idx + j < spec_width {
                label_line[start_idx + j] = c;
            }
        }
    };

    if num_cols >= 16 && spec_width >= 16 {
        let min_freq = 20.0f32;
        let max_freq = if state.max_frequency >= 2000.0 {
            state.max_frequency
        } else {
            20000.0
        };
        let log_denom = (max_freq / min_freq).ln().max(0.001);
        let col_at = |freq: f32| -> usize {
            let frac = ((freq / min_freq).ln() / log_denom).clamp(0.0, 1.0);
            ((num_cols.saturating_sub(1) as f32) * frac).round() as usize
        };

        write_label(0, "20Hz");
        write_label(col_at(100.0), "100Hz");
        write_label(col_at(1000.0), "1kHz");
        if max_freq >= 6000.0 {
            write_label(col_at(5000.0), "5kHz");
        }
        let max_label = if max_freq >= 1000.0 {
            format!("{:.0}kHz", max_freq / 1000.0)
        } else {
            format!("{:.0}Hz", max_freq)
        };
        write_label(num_cols.saturating_sub(1), &max_label);
    }

    let label_str: String = label_line.into_iter().collect();
    spec_lines.push(Line::from(vec![Span::styled(
        label_str,
        Style::default().fg(Color::DarkGray),
    )]));

    let spec_paragraph = Paragraph::new(spec_lines).block(
        Block::default()
            .title("128-Band Spectrum Analyzer")
            .borders(Borders::ALL),
    );
    f.render_widget(spec_paragraph, layout.spectrum_rect);

    // 5. Timeline Gauge
    let display_secs = state.scrub_target_seconds.unwrap_or(state.current_seconds);
    let progress = if state.duration_seconds > 0.0 {
        (display_secs / state.duration_seconds).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let force_hours = state.duration_seconds >= 3600.0 || display_secs >= 3600.0;
    let status_icon = if state.is_paused { "⏸" } else { "▶" };
    let gauge_label = if state.duration_seconds <= 0.0 {
        format!(
            "{} {} / LIVE",
            status_icon,
            crate::state::format_timeline_time(display_secs, force_hours)
        )
    } else {
        format!(
            "{} {} / {}",
            status_icon,
            crate::state::format_timeline_time(display_secs, force_hours),
            crate::state::format_timeline_time(state.duration_seconds, force_hours)
        )
    };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Rgb(30, 30, 30)))
        .percent((progress * 100.0).round() as u16)
        .label(gauge_label);

    f.render_widget(gauge, layout.timeline_rect);

    // 6. Instructions & Active OSD Notification
    let hints = "q/Esc: Quit │ Space: Pause │ ←/→: Seek │ n/p: Track │ v/Tab: View │ u: Upmix │ d: Device │ [/]: Split";
    let footer_line = if state.osd_timer > 0.0
        && let Some(osd) = &state.osd_text
    {
        let single_line_osd = osd.replace('\n', " — ");
        let badge = format!(" {} ", single_line_osd);
        let badge_w = str_display_width(&badge);
        let rem_w = (layout.footer_rect.width as usize).saturating_sub(badge_w + 2);
        Line::from(vec![
            Span::styled(
                badge,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                truncate_to_display_width(hints, rem_w),
                Style::default().add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![Span::styled(
            truncate_to_display_width(hints, layout.footer_rect.width as usize),
            Style::default().add_modifier(Modifier::ITALIC),
        )])
    };
    f.render_widget(Paragraph::new(footer_line), layout.footer_rect);

    // 7. Device Picker Modal Overlay
    if let Some(modal_area) = layout.device_picker_rect {
        f.render_widget(Clear, modal_area);

        let items: Vec<ListItem> = if state.available_audio_devices.is_empty() {
            vec![
                ListItem::new("  (No audio devices detected)")
                    .style(Style::default().fg(Color::DarkGray)),
            ]
        } else {
            state
                .available_audio_devices
                .iter()
                .enumerate()
                .map(|(idx, dev)| {
                    let is_active = state
                        .selected_audio_device
                        .as_deref()
                        .is_some_and(|sel| sel == dev);
                    let prefix = if is_active { "✓ " } else { "  " };
                    let style = if idx == state.tui_device_picker_cursor {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if is_active {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(format!("{}{}", prefix, dev)).style(style)
                })
                .collect()
        };

        let modal_title = if state.is_mic_input {
            "Select Audio Input Device"
        } else {
            "Select Audio Output Device"
        };
        let list =
            List::new(items).block(Block::default().title(modal_title).borders(Borders::ALL));

        let mut list_state = ListState::default();
        if !state.available_audio_devices.is_empty() {
            list_state.select(Some(
                state
                    .tui_device_picker_cursor
                    .min(state.available_audio_devices.len() - 1),
            ));
        }
        f.render_stateful_widget(list, modal_area, &mut list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn test_tui_renders_tracker_pattern_and_surround_eq_views() {
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new("cyber_symphony.it".to_string());
        state.file_loaded = true;
        state.is_paused = false;
        state.module_type = "Impulse Tracker".to_string();
        state.bpm = 140;
        state.speed = 6;
        state.num_patterns = 12;
        state.current_tracker_order = 1;
        state.current_tracker_row = 1;
        state.tracker_patterns_by_order = std::sync::Arc::new(vec![
            vec!["C-5 01 .. .00".to_string(); 4],
            vec![
                "C-4 01 64 .00 | --- .. .. .00".to_string(),
                "D-4 02 64 F06 | G-3 03 48 .00".to_string(),
                "E-4 01 64 .00 | --- .. .. .00".to_string(),
            ],
        ]);
        state.channel_vus = vec![1.0, 0.5, 0.25, 0.0];
        state.peak_vus = vec![1.0, 0.75, 0.3, 0.0];
        state.spectrum_data[0] = 100.0;
        state.spectrum_peaks[0] = 100.0;

        // 1. Default view for tracker module without lyrics is Tracker Pattern
        assert_eq!(state.effective_tui_center_view(), TuiCenterView::Tracker);
        terminal.draw(|f| draw(f, &state)).unwrap();
        let screen = buffer_to_string(&terminal);
        assert!(screen.contains("Tracker Pattern (Ord 02/12"));
        assert!(screen.contains("D-4 02 64 F06"));
        assert!(
            screen.contains("▄"),
            "0 dBFS full-scale peak cap must render"
        );

        // 2. Cycle to Heatmap History
        assert_eq!(state.cycle_tui_center_view(), TuiCenterView::Heatmap);
        terminal.draw(|f| draw(f, &state)).unwrap();
        let screen_hm = buffer_to_string(&terminal);
        assert!(screen_hm.contains("Heatmap History"));
        assert!(screen_hm.contains("View: Spectrogram Heatmap"));

        // 3. Cycle to Surround EQ
        assert_eq!(state.cycle_tui_center_view(), TuiCenterView::SurroundEq);
        terminal.draw(|f| draw(f, &state)).unwrap();
        let screen_eq = buffer_to_string(&terminal);
        assert!(screen_eq.contains("Surround Upmix & EQ"));
        assert!(screen_eq.contains("Sub XO:"));
    }

    #[test]
    fn test_tui_heatmap_downsamples_full_1024_bin_spectrum() {
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new("sweep.flac".to_string());
        state.file_loaded = true;
        // Put high energy at bin 900 (near the top of the 1024-bin spectrum, well beyond heatmap_width)
        for row in &mut state.spectrum_history {
            row[900] = 95.0;
        }

        terminal.draw(|f| draw(f, &state)).unwrap();
        let layout = compute_tui_layout(Rect::new(0, 0, 120, 32), &state, 10);
        let buf = terminal.backend().buffer();

        // Verify that the right side of the heatmap panel contains white-hot RGB(255,255,255) from bin 900
        let mut found_white_hot = false;
        for y in (layout.center_rect.y + 1)..(layout.center_rect.bottom() - 1) {
            for x in (layout.center_rect.x + layout.center_rect.width / 2)
                ..(layout.center_rect.right() - 1)
            {
                if buf[(x, y)].fg == Color::Rgb(255, 255, 255) {
                    found_white_hot = true;
                    break;
                }
            }
        }
        assert!(
            found_white_hot,
            "High-frequency bins (e.g. bin 900/1024) must be downsampled into the visible heatmap width"
        );
    }

    #[test]
    fn test_tui_split_ratio_resizes_top_and_spectrum_panels() {
        let area = Rect::new(0, 0, 120, 40);
        let mut state = AppState::new("track.flac".to_string());

        let default_layout = compute_tui_layout(area, &state, 10);
        state.panel_split_ratio = 0.30;
        let shrunk_layout = compute_tui_layout(area, &state, 10);
        state.panel_split_ratio = 0.70;
        let expanded_layout = compute_tui_layout(area, &state, 10);

        assert!(shrunk_layout.info_rect.height < default_layout.info_rect.height);
        assert!(expanded_layout.info_rect.height > default_layout.info_rect.height);
        assert!(shrunk_layout.spectrum_rect.height > default_layout.spectrum_rect.height);
    }

    #[test]
    fn test_tui_channels_multicolumn_bars_and_tracker_vs_surround_labels() {
        let backend = TestBackend::new(180, 40);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new("ive_got_the_power.mod".to_string());
        state.file_loaded = true;
        state.tracker_channels = Some(4);
        state.source_channels = 2;
        state.hardware_channels = 2;
        state.num_channels = 6;
        state.channel_vus = vec![0.75, 0.65, 0.85, 0.90, 0.70, 0.75];
        state.peak_vus = vec![0.85, 0.75, 0.95, 0.98, 0.80, 0.85];

        terminal.draw(|f| draw(f, &state)).unwrap();
        let screen = buffer_to_string(&terminal);

        // Verify title, dBFS ruler, and wide multi-column bars (at least 4 consecutive '█' per bar)
        assert!(screen.contains("Channels (4 Trk + L/R)"));
        assert!(screen.contains("dBFS"));
        assert!(
            screen.contains("████"),
            "Expected multi-column wide VU bars instead of single-column bars"
        );

        // Verify 5.1 surround speaker labels when not a tracker module
        state.tracker_channels = None;
        state.source_channels = 2;
        state.hardware_channels = 6;
        terminal.draw(|f| draw(f, &state)).unwrap();
        let surround_screen = buffer_to_string(&terminal);
        assert!(surround_screen.contains("Channels (5.1 Upmix)"));
        assert!(surround_screen.contains("LFE"));
        assert!(surround_screen.contains("Ls"));
        assert!(surround_screen.contains("Rs"));
    }
}
