use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::state::AppState;

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

pub fn draw(f: &mut Frame, state: &AppState) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(15), // Top: VUs, Heatmap, Meta
            Constraint::Min(10),    // Middle: HUGE Spectrum Analyzer
            Constraint::Length(1),  // Bottom: Timeline Gauge
            Constraint::Length(1)   // Instructions
        ].as_ref())
        .split(f.area());

    let width = f.area().width;
    let top_constraints = if width < 80 {
        vec![
            Constraint::Percentage(40), // VUs
            Constraint::Percentage(0),  // Hide Heatmap
            Constraint::Percentage(60), // Metadata
        ]
    } else if width < 110 {
        vec![
            Constraint::Percentage(30), // VUs
            Constraint::Percentage(40), // Heatmap
            Constraint::Percentage(30), // Metadata
        ]
    } else {
        vec![
            Constraint::Percentage(30), // VUs
            Constraint::Percentage(50), // Heatmap
            Constraint::Percentage(20), // Metadata
        ]
    };

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(top_constraints)
        .split(main_chunks[0]);

    // 1. Layered Channel VUs with Peak Decay
    let vu_height = top_chunks[0].height.saturating_sub(2) as usize; 
    let vu_width = top_chunks[0].width.saturating_sub(2) as usize;
    let chars_per_vu = (vu_width / state.channel_vus.len().max(1)).max(1);

    let mut vu_lines = Vec::new();
    
    // Render VUs top to bottom, leaving 1 line at the bottom for labels
    for row in (0..vu_height.saturating_sub(1)).rev() {
        let mut spans = Vec::new();
        let threshold = (row as f32) / ((vu_height - 1) as f32);
        let next_threshold = ((row + 1) as f32) / ((vu_height - 1) as f32);

        for i in 0..state.channel_vus.len() {
            let vu = state.channel_vus[i];
            let peak = state.peak_vus[i];

            let symbol = if peak > 0.01 && peak >= threshold && peak < next_threshold {
                "▄"
            } else if vu >= threshold || (peak > threshold && peak > 0.01) {
                "█"
            } else {
                " "
            };

            let padded_symbol = match chars_per_vu {
                1 => format!("{}", symbol),
                2 => format!("{} ", symbol),
                _ => format!(" {}{}", symbol, " ".repeat(chars_per_vu - 2)),
            };

            let mut style = Style::default();
            if peak > 0.01 && peak >= threshold && peak < next_threshold {
                style = Style::default().fg(Color::White); // Peak cap
            } else if peak > threshold && vu < threshold && peak > 0.01 {
                let diff = (peak - vu).max(0.001);
                let dist = threshold - vu;
                let fade_ratio = (dist / diff).clamp(0.0, 1.0);
                style = Style::default().fg(val_to_color(100.0 - (fade_ratio * 100.0)));
            } else if vu >= threshold && vu > 0.01 {
                style = Style::default().fg(val_to_color(vu * 100.0));
            }

            spans.push(Span::styled(padded_symbol, style));
        }
        vu_lines.push(Line::from(spans));
    }

    // VU Labels
    let mut vu_label_spans = Vec::new();
    for i in 0..state.channel_vus.len() {
        let label = format!("{}", i + 1);
        let show = chars_per_vu >= 3 || (i % 2 == 0);
        
        let padded_label = if !show {
            " ".repeat(chars_per_vu)
        } else {
            match chars_per_vu {
                1 => String::from(if label.len() == 1 { &label[..1] } else { "+" }),
                2 => format!("{:>2}", label),
                3 => format!("{:^3}", label),
                _ => format!("{:^width$}", label, width=chars_per_vu),
            }
        };
        vu_label_spans.push(Span::styled(padded_label, Style::default().fg(Color::DarkGray)));
    }
    vu_lines.push(Line::from(vu_label_spans));

    let vu_paragraph = Paragraph::new(vu_lines)
        .block(Block::default().title("Channels").borders(Borders::ALL));
    f.render_widget(vu_paragraph, top_chunks[0]);

    // 2. High-End Spectrogram Heatmap
    if top_chunks[1].width > 2 && top_chunks[1].height > 2 {
        let heatmap_width = top_chunks[1].width.saturating_sub(2) as usize;
        let heatmap_height = top_chunks[1].height.saturating_sub(2) as usize;
        let chars_per_bin = (heatmap_width / 128).max(1);
        let bin_str = "▀".repeat(chars_per_bin);

        let mut heatmap_lines = Vec::new();
        let history_len = state.spectrum_history.len();
        let total_history_lines = history_len / 2;
        let start_line = total_history_lines.saturating_sub(heatmap_height);
        
        for cell_y in start_line..total_history_lines {
            let mut spans = Vec::new();
            let top_row_idx = cell_y * 2;
            let bottom_row_idx = cell_y * 2 + 1;

            if top_row_idx < history_len && bottom_row_idx < history_len {
                let top_row = &state.spectrum_history[top_row_idx];
                let bottom_row = &state.spectrum_history[bottom_row_idx];

                for x in 0..top_row.len() {
                    let fg_col = val_to_color(top_row[x]);
                    let bg_col = val_to_color(bottom_row[x]);

                    spans.push(Span::styled(
                        bin_str.clone(), 
                        Style::default().fg(fg_col).bg(bg_col)
                    ));
                }
            }
            heatmap_lines.push(Line::from(spans));
        }

        let heatmap_paragraph = Paragraph::new(heatmap_lines)
            .block(Block::default().title("Heatmap History").borders(Borders::ALL));
        f.render_widget(heatmap_paragraph, top_chunks[1]);
    }


    // 3. Metadata
    let current_path_str = if state.playlist_index < state.playlist.len() {
        state.playlist[state.playlist_index].clone()
    } else {
        state.song_title.clone()
    };
    let is_network = current_path_str.starts_with("http://") || current_path_str.starts_with("https://");
    
    let scroll_text = |text: &str, max_len: usize| -> String {
        let char_count = text.chars().count();
        if char_count > max_len {
            let max_scroll = char_count - max_len;
            let scroll_duration = max_scroll as f32 / 3.0; // 3 characters per second
            let total_period = 2.0 + scroll_duration + 2.0 + scroll_duration;
            let t = (state.current_seconds as f32) % total_period;
            
            let offset = if t < 2.0 {
                0
            } else if t < 2.0 + scroll_duration {
                let progress = (t - 2.0) / scroll_duration;
                (progress * max_scroll as f32) as usize
            } else if t < 2.0 + scroll_duration + 2.0 {
                max_scroll
            } else {
                let progress = (t - (2.0 + scroll_duration + 2.0)) / scroll_duration;
                max_scroll.saturating_sub((progress * max_scroll as f32) as usize)
            };
            
            let offset = offset.clamp(0, max_scroll);
            text.chars().skip(offset).take(max_len).collect()
        } else {
            text.to_string()
        }
    };

    let file_name = if is_network {
        state.song_title.clone()
    } else {
        std::path::Path::new(&state.song_title).file_name().unwrap_or_default().to_string_lossy().to_string()
    };
    let file_dir = if is_network {
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

    let width = top_chunks[2].width.saturating_sub(2) as usize; // inside block width
    
    let mut lines = Vec::new();
    
    // 1. File Name / Title (bold, styled)
    lines.push(Line::from(vec![
        Span::styled(scroll_text(&file_name, width), Style::default().add_modifier(Modifier::BOLD))
    ]));

    // 2. Artist
    lines.push(Line::from(vec![
        Span::raw("Artist: "),
        Span::raw(&state.artist)
    ]));

    // 3. Path / URL (scrollable)
    let path_label = if is_network { "URL:    " } else { "Path:   " };
    lines.push(Line::from(vec![
        Span::raw(path_label),
        Span::raw(scroll_text(&file_dir, width.saturating_sub(8)))
    ]));

    // 4. Type
    lines.push(Line::from(vec![
        Span::raw("Type:   "),
        Span::raw(&state.module_type)
    ]));

    // 4b. Visualization
    let vis_name = crate::state::VISUALIZERS.get(state.current_visualizer_idx)
        .map(|v| v.name)
        .unwrap_or("Unknown");
    lines.push(Line::from(vec![
        Span::raw("Vis:    "),
        Span::raw(vis_name)
    ]));

    // 5. BPM (if available)
    if state.bpm > 0 {
        lines.push(Line::from(vec![
            Span::raw("BPM:    "),
            Span::raw(state.bpm.to_string())
        ]));
    }

    // 6. Speed (if available)
    if state.speed > 0 {
        lines.push(Line::from(vec![
            Span::raw("Speed:  "),
            Span::raw(state.speed.to_string())
        ]));
    }

    // 7. Channels
    lines.push(Line::from(vec![
        Span::raw("Chans:  "),
        Span::raw(state.num_channels.to_string())
    ]));

    // 8. Sample Rate
    lines.push(Line::from(vec![
        Span::raw("Rate:   "),
        Span::raw(format!("{} Hz", state.current_sample_rate as u32))
    ]));

    // 9. Bitrate
    lines.push(Line::from(vec![
        Span::raw("Bitr:   "),
        Span::raw(state.bitrate.map(|b| format!("{} kbps", b)).unwrap_or_else(|| "Unknown".to_string()))
    ]));

    // 10. Length
    lines.push(Line::from(vec![
        Span::raw("Length: "),
        Span::raw(if state.duration_seconds <= 0.0 { "LIVE".to_string() } else { format!("{:.1}s", state.duration_seconds) })
    ]));

    // 11. Next Song (at the bottom, scrollable)
    if state.playlist_index + 1 < state.playlist.len() {
        let next_path = std::path::Path::new(&state.playlist[state.playlist_index + 1]);
        let next_song = next_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        lines.push(Line::from(vec![
            Span::styled("Next:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(scroll_text(&next_song, width.saturating_sub(8)), Style::default().fg(Color::DarkGray))
        ]));
    }

    let title_text = if state.playlist.len() > 1 {
        format!("Track Info ({}/{})", state.playlist_index + 1, state.playlist.len())
    } else {
        "Track Info".to_string()
    };

    let meta_paragraph = Paragraph::new(lines)
        .block(Block::default().title(title_text).borders(Borders::ALL));

    f.render_widget(meta_paragraph, top_chunks[2]);


    // 4. HUGE Spectrum Analyzer
    let spec_height = main_chunks[1].height.saturating_sub(2) as usize;
    let spec_width = main_chunks[1].width.saturating_sub(2) as usize;
    let chars_per_spec = (spec_width / state.spectrum_data.len().max(1)).max(1);
    let mut spec_lines = Vec::new();

    for row in (0..spec_height.saturating_sub(1)).rev() {
        let mut spans = Vec::new();
        let threshold = (row as f32) / ((spec_height - 1) as f32) * 100.0;
        let next_threshold = ((row + 1) as f32) / ((spec_height - 1) as f32) * 100.0;

        for i in 0..state.spectrum_data.len() {
            let val = state.spectrum_data[i];
            let peak = state.spectrum_peaks[i];

            let symbol = if peak >= threshold && peak < next_threshold {
                "▄"
            } else if val >= threshold || peak > threshold {
                "█"
            } else {
                " "
            };

            let padded_symbol = match chars_per_spec {
                1 => format!("{}", symbol),
                2 => format!("{} ", symbol),
                _ => format!(" {}{}", symbol, " ".repeat(chars_per_spec.saturating_sub(2))),
            };

            let mut style = Style::default();
            if peak >= threshold && peak < next_threshold {
                style = Style::default().fg(Color::White); // Peak cap
            } else if peak > threshold && val < threshold {
                // Color fade out trailing effect
                let diff = (peak - val).max(0.001);
                let dist = threshold - val;
                let fade_ratio = (dist / diff).clamp(0.0, 1.0);
                style = Style::default().fg(val_to_color(100.0 - (fade_ratio * 100.0))); 
            } else if val >= threshold {
                style = Style::default().fg(val_to_color(val)); // Solid bright bar
            }

            spans.push(Span::styled(padded_symbol, style));
        }
        spec_lines.push(Line::from(spans));
    }

    // Spectrum Labels (Overlay onto a single string to prevent truncation)
    let mut label_line = vec![' '; spec_width];
    let mut write_label = |bin: usize, text: &str| {
        let mut start_idx = bin * chars_per_spec;
        if start_idx + text.len() > spec_width {
            start_idx = spec_width.saturating_sub(text.len());
        }
        for (j, c) in text.chars().enumerate() {
            if start_idx + j < spec_width {
                label_line[start_idx + j] = c;
            }
        }
    };
    
    if state.spectrum_data.len() >= 128 {
        write_label(0, "0Hz");
        write_label(128 / 4, "2.5kHz");
        write_label(128 / 2, "5kHz");
        write_label(128 * 3 / 4, "7.5kHz");
        write_label(127, "10kHz");
    }
    
    let label_str: String = label_line.into_iter().collect();
    spec_lines.push(Line::from(vec![Span::styled(label_str, Style::default().fg(Color::DarkGray))]));

    let spec_paragraph = Paragraph::new(spec_lines)
        .block(Block::default().title("128-Band Spectrum Analyzer").borders(Borders::ALL));
    f.render_widget(spec_paragraph, main_chunks[1]);


    // 5. Timeline Gauge
    let progress = if state.duration_seconds > 0.0 {
        (state.current_seconds / state.duration_seconds).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let gauge = ratatui::widgets::Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Rgb(30, 30, 30)))
        .percent((progress * 100.0) as u16)
        .label(format!("{:.1}s / {:.1}s", state.current_seconds, state.duration_seconds));
    
    f.render_widget(gauge, main_chunks[2]);

    // 6. Instructions
    let instructions = Paragraph::new("Press 'q' to quit | Space to pause | ⬅️  ➡️  to scrub timeline | 'd' for device picker")
        .style(Style::default().add_modifier(Modifier::ITALIC));
    f.render_widget(instructions, main_chunks[3]);

    // 7. Device Picker Modal Overlay
    if state.show_tui_device_picker {
        use ratatui::widgets::{Clear, List, ListItem};
        
        let area = f.area();
        let modal_width = 50.min(area.width.saturating_sub(4));
        let modal_height = (state.available_audio_devices.len() as u16 + 2).min(area.height.saturating_sub(4));
        
        let modal_area = ratatui::layout::Rect::new(
            (area.width - modal_width) / 2,
            (area.height - modal_height) / 2,
            modal_width,
            modal_height,
        );
        
        f.render_widget(Clear, modal_area);
        
        let items: Vec<ListItem> = state.available_audio_devices.iter().enumerate().map(|(idx, dev)| {
            let style = if idx == state.tui_device_picker_cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(dev.clone()).style(style)
        }).collect();
        
        let list = List::new(items)
            .block(Block::default().title("Select Audio Output Device").borders(Borders::ALL));
            
        f.render_widget(list, modal_area);
    }
}
