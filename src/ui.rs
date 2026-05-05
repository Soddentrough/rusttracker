use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Table, Row},
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

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30), // VUs
            Constraint::Percentage(50), // Heatmap
            Constraint::Percentage(20), // Metadata
        ].as_ref())
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


    // 3. Metadata
    let meta_table = Table::new(
        vec![
            Row::new(vec!["Title".to_string(), state.song_title.clone()]),
            Row::new(vec!["Artist".to_string(), state.artist.clone()]),
            Row::new(vec!["Type".to_string(), state.module_type.clone()]),
            Row::new(vec!["BPM".to_string(), state.bpm.to_string()]),
            Row::new(vec!["Speed".to_string(), state.speed.to_string()]),
            Row::new(vec!["Channels".to_string(), state.num_channels.to_string()]),
            Row::new(vec!["Length".to_string(), format!("{:.1}s", state.duration_seconds)]),
        ],
        [Constraint::Percentage(40), Constraint::Percentage(60)].as_ref()
    ).block(Block::default().title("Track Info").borders(Borders::ALL));

    f.render_widget(meta_table, top_chunks[2]);


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
    let instructions = Paragraph::new("Press 'q' to quit | Space to pause | ⬅️  ➡️  to scrub timeline")
        .style(Style::default().add_modifier(Modifier::ITALIC));
    f.render_widget(instructions, main_chunks[3]);
}
