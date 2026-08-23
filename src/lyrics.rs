use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    pub time_seconds: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    pub file_name: Option<String>,
    pub lines: Vec<LyricLine>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub offset_ms: i64,
}

impl Lyrics {
    /// Parse raw LRC string content into a structured `Lyrics` object.
    pub fn parse(content: &str) -> Option<Self> {
        let content = content.trim_start_matches('\u{feff}'); // Strip UTF-8 BOM if present
        let mut lines = Vec::new();
        let mut title = None;
        let mut artist = None;
        let mut album = None;
        let mut offset_ms: i64 = 0;

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            // Extract all [tags] at the beginning of or within the line
            let mut timestamps = Vec::new();
            let mut remaining = line;

            while remaining.starts_with('[') {
                if let Some(close_bracket) = remaining.find(']') {
                    let tag_content = &remaining[1..close_bracket].trim();
                    
                    // Check if it's a metadata tag (e.g., [ti:Song Title], [ar:Artist], [offset:500])
                    if let Some((key, val)) = tag_content.split_once(':') {
                        let key = key.trim().to_lowercase();
                        let val = val.trim();
                        
                        match key.as_str() {
                            "ti" => title = Some(val.to_string()),
                            "ar" => artist = Some(val.to_string()),
                            "al" => album = Some(val.to_string()),
                            "offset" => {
                                if let Ok(parsed_offset) = val.parse::<i64>() {
                                    offset_ms = parsed_offset;
                                }
                            }
                            _ => {
                                // Try parsing as timestamp (e.g. mm:ss.xx or mm:ss)
                                if let Some(sec) = parse_time_tag(tag_content) {
                                    timestamps.push(sec);
                                }
                            }
                        }
                    } else if let Some(sec) = parse_time_tag(tag_content) {
                        timestamps.push(sec);
                    }

                    remaining = remaining[close_bracket + 1..].trim_start();
                } else {
                    break;
                }
            }

            // Strip any remaining inline enhanced LRC tags (e.g., <00:12.34>)
            let cleaned_text = strip_inline_tags(remaining);

            for ts in timestamps {
                lines.push(LyricLine {
                    time_seconds: ts,
                    text: cleaned_text.clone(),
                });
            }
        }

        if lines.is_empty() {
            return None;
        }

        // Apply overall offset: positive offset moves timestamps (makes lyrics appear sooner or accounts for track delay)
        let offset_sec = (offset_ms as f64) / 1000.0;
        for line in &mut lines {
            line.time_seconds = (line.time_seconds + offset_sec).max(0.0);
        }

        // Sort lines chronologically
        lines.sort_by(|a, b| a.time_seconds.partial_cmp(&b.time_seconds).unwrap_or(std::cmp::Ordering::Equal));

        Some(Lyrics {
            file_name: None,
            lines,
            title,
            artist,
            album,
            offset_ms,
        })
    }

    /// Finds the index of the currently active lyric line for the given timestamp in seconds.
    pub fn find_current_line_idx(&self, current_time: f64) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        if current_time < self.lines[0].time_seconds {
            return None;
        }

        match self.lines.binary_search_by(|line| {
            line.time_seconds.partial_cmp(&current_time).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(exact) => {
                // If there are multiple identical timestamps, return the last one
                let mut idx = exact;
                while idx + 1 < self.lines.len() && self.lines[idx + 1].time_seconds <= current_time {
                    idx += 1;
                }
                Some(idx)
            }
            Err(next_idx) => Some(next_idx.saturating_sub(1)),
        }
    }
}

/// Parses standard LRC time tags such as "01:23.45", "01:23.456", "01:23:45", "01:23", or "01:02:03.45" into seconds.
fn parse_time_tag(tag: &str) -> Option<f64> {
    let parts: Vec<&str> = tag.split(':').collect();
    match parts.len() {
        2 => {
            // [mm:ss.xx] or [mm:ss] or [mm:ss:xx]
            let minutes: f64 = parts[0].trim().parse().ok()?;
            let sec_part = parts[1].trim();

            if let Some((sec_str, frac_str)) = sec_part.split_once('.') {
                let seconds: f64 = sec_str.parse().ok()?;
                let frac: f64 = format!("0.{}", frac_str).parse().unwrap_or(0.0);
                Some(minutes * 60.0 + seconds + frac)
            } else if let Some((sec_str, frac_str)) = sec_part.split_once(',') {
                let seconds: f64 = sec_str.parse().ok()?;
                let frac: f64 = format!("0.{}", frac_str).parse().unwrap_or(0.0);
                Some(minutes * 60.0 + seconds + frac)
            } else {
                let seconds: f64 = sec_part.parse().ok()?;
                Some(minutes * 60.0 + seconds)
            }
        }
        3 => {
            // [hh:mm:ss.xx] or legacy [mm:ss:xx] (where xx is centiseconds)
            let p1: f64 = parts[0].trim().parse().ok()?;
            let p2: f64 = parts[1].trim().parse().ok()?;
            let p3_str = parts[2].trim();

            if p3_str.contains('.') || p3_str.contains(',') {
                // Definitely [hh:mm:ss.frac]
                let (sec_str, frac_str) = p3_str.split_once('.').or_else(|| p3_str.split_once(',')).unwrap();
                let seconds: f64 = sec_str.parse().ok()?;
                let frac: f64 = format!("0.{}", frac_str).parse().unwrap_or(0.0);
                Some(p1 * 3600.0 + p2 * 60.0 + seconds + frac)
            } else if let Ok(p3) = p3_str.parse::<f64>() {
                // If p3 is 2 digits and p1 < 60, it's typically [mm:ss:centiseconds]
                if p3_str.len() <= 2 && p1 < 60.0 && p2 < 60.0 {
                    Some(p1 * 60.0 + p2 + p3 / 100.0)
                } else if p3_str.len() == 3 && p1 < 60.0 && p2 < 60.0 {
                    // [mm:ss:milliseconds]
                    Some(p1 * 60.0 + p2 + p3 / 1000.0)
                } else {
                    // [hh:mm:ss]
                    Some(p1 * 3600.0 + p2 * 60.0 + p3)
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Strips inline enhanced LRC tags like `<00:12.34>` or `<12:34>` from text and normalizes spaces.
fn strip_inline_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == '>' {
                    closed = true;
                    break;
                }
                tag.push(inner);
            }
            // If it wasn't closed or not a valid inline tag, keep original
            if !closed {
                result.push('<');
                result.push_str(&tag);
            }
        } else {
            result.push(ch);
        }
    }

    // Collapse multiple consecutive whitespace characters
    let mut normalized = String::with_capacity(result.len());
    let mut last_was_space = false;
    for ch in result.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }

    normalized
}

/// Normalizes a path string by stripping URI schemas (like `file://`) and decoding URL-encoded characters.
pub fn normalize_path_str(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(stripped) = s.strip_prefix("file://") {
        s = stripped;
    }
    let mut decoded = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i+1..i+3]).unwrap_or(""), 16) {
                decoded.push(val as char);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i] as char);
        i += 1;
    }
    decoded
}

fn clean_song_stem(stem: &str) -> String {
    let mut s = stem.to_lowercase();
    s = s.replace('_', " ").replace('-', " ").replace('.', " ");
    for tag in &["(instrumental)", "(vocals)", "(karaoke)", "(official)", "[flac]", "[mp3]", "(audio)", "(lyrics)"] {
        s = s.replace(tag, "");
    }
    s.retain(|c| c.is_alphanumeric() || c.is_whitespace());
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Discovers an .lrc sidecar file for a given audio file path.
pub fn find_sidecar_lrc_path(audio_path: &str) -> Option<PathBuf> {
    if audio_path.starts_with("http://") || audio_path.starts_with("https://") {
        return None;
    }

    let normalized_str = normalize_path_str(audio_path);
    let raw_path = PathBuf::from(&normalized_str);
    let path = raw_path.canonicalize().unwrap_or(raw_path);

    // Candidate 1: Same path with .lrc extension (e.g. song.flac -> song.lrc)
    let lrc_path = path.with_extension("lrc");
    if lrc_path.is_file() {
        return Some(lrc_path);
    }

    // Candidate 2: Same path with .LRC extension
    let lrc_upper = path.with_extension("LRC");
    if lrc_upper.is_file() {
        return Some(lrc_upper);
    }

    // Candidate 3: Appended .lrc (e.g. song.flac -> song.flac.lrc)
    let mut appended_lrc = path.clone();
    if let Some(fname) = appended_lrc.file_name().and_then(|n| n.to_str()) {
        appended_lrc.set_file_name(format!("{}.lrc", fname));
        if appended_lrc.is_file() {
            return Some(appended_lrc);
        }
    }

    // Candidate 4: Appended .LRC
    let mut appended_upper = path.clone();
    if let Some(fname) = appended_upper.file_name().and_then(|n| n.to_str()) {
        appended_upper.set_file_name(format!("{}.LRC", fname));
        if appended_upper.is_file() {
            return Some(appended_upper);
        }
    }

    // Candidate 5: Directory scans in media folder and local lyrics/ subfolder
    if let (Some(parent), Some(stem)) = (path.parent(), path.file_stem()) {
        let stem_raw = stem.to_string_lossy();
        let stem_clean = clean_song_stem(&stem_raw);
        
        let search_dirs = [
            parent.to_path_buf(),
            parent.join("lyrics"),
            parent.join("Lyrics"),
        ];

        for dir in &search_dirs {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_file() {
                        if let Some(ext) = entry_path.extension() {
                            if ext.eq_ignore_ascii_case("lrc") {
                                if let Some(entry_stem) = entry_path.file_stem() {
                                    let entry_stem_raw = entry_stem.to_string_lossy();
                                    let entry_stem_clean = clean_song_stem(&entry_stem_raw);
                                    if entry_stem_raw.eq_ignore_ascii_case(&stem_raw)
                                        || (!stem_clean.is_empty() && entry_stem_clean == stem_clean)
                                        || (!stem_clean.is_empty() && !entry_stem_clean.is_empty() && (stem_clean.starts_with(&entry_stem_clean) || entry_stem_clean.starts_with(&stem_clean)))
                                    {
                                        return Some(entry_path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Loads and parses an .lrc sidecar lyrics file for the specified audio path if present.
pub fn load_lyrics_for_file(audio_path: &str) -> Option<Lyrics> {
    let lrc_path = find_sidecar_lrc_path(audio_path)?;
    let content = fs::read_to_string(&lrc_path).ok()?;
    let mut lyrics = Lyrics::parse(&content)?;
    lyrics.file_name = lrc_path.file_name().map(|f| f.to_string_lossy().into_owned());
    Some(lyrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_lrc() {
        let lrc_data = r#"
[ti:Another Love]
[ar:Tom Odell]
[al:Long Way Down]
[00:21.06] I wanna take you somewhere so you know I care
[00:25.61] But it's so cold, and I don't know where
[00:29.69] I brought you daffodils in a pretty string
[01:34.83]
[01:54.76] And if somebody hurts you, I wanna fight
"#;
        let lyrics = Lyrics::parse(lrc_data).expect("Should parse valid LRC");
        assert_eq!(lyrics.title.as_deref(), Some("Another Love"));
        assert_eq!(lyrics.artist.as_deref(), Some("Tom Odell"));
        assert_eq!(lyrics.album.as_deref(), Some("Long Way Down"));
        assert_eq!(lyrics.lines.len(), 5);

        assert!((lyrics.lines[0].time_seconds - 21.06).abs() < 1e-4);
        assert_eq!(lyrics.lines[0].text, "I wanna take you somewhere so you know I care");

        assert!((lyrics.lines[3].time_seconds - 94.83).abs() < 1e-4);
        assert_eq!(lyrics.lines[3].text, "");
    }

    #[test]
    fn test_multi_timestamp_and_inline_tags() {
        let lrc_data = r#"
[00:10.00][00:20.00] <00:10.00> Repeated <00:11.00> line <00:12.00> text
[00:30.00] Third line
"#;
        let lyrics = Lyrics::parse(lrc_data).expect("Should parse multi-timestamp LRC");
        assert_eq!(lyrics.lines.len(), 3);
        assert!((lyrics.lines[0].time_seconds - 10.0).abs() < 1e-4);
        assert_eq!(lyrics.lines[0].text, "Repeated line text");
        assert!((lyrics.lines[1].time_seconds - 20.0).abs() < 1e-4);
        assert_eq!(lyrics.lines[1].text, "Repeated line text");
        assert!((lyrics.lines[2].time_seconds - 30.0).abs() < 1e-4);
    }

    #[test]
    fn test_offset() {
        let lrc_data = r#"
[offset:500]
[00:10.00] Offset line
"#;
        let lyrics = Lyrics::parse(lrc_data).expect("Should parse offset");
        assert_eq!(lyrics.offset_ms, 500);
        assert!((lyrics.lines[0].time_seconds - 10.5).abs() < 1e-4);
    }

    #[test]
    fn test_find_current_line_idx() {
        let lrc_data = r#"
[00:10.00] Line 1
[00:20.00] Line 2
[00:30.00] Line 3
"#;
        let lyrics = Lyrics::parse(lrc_data).unwrap();
        assert_eq!(lyrics.find_current_line_idx(5.0), None);
        assert_eq!(lyrics.find_current_line_idx(10.0), Some(0));
        assert_eq!(lyrics.find_current_line_idx(15.0), Some(0));
        assert_eq!(lyrics.find_current_line_idx(20.0), Some(1));
        assert_eq!(lyrics.find_current_line_idx(25.0), Some(1));
        assert_eq!(lyrics.find_current_line_idx(30.0), Some(2));
        assert_eq!(lyrics.find_current_line_idx(40.0), Some(2));
    }

    #[test]
    fn test_various_timestamp_formats() {
        let lrc_data = r#"
[01:05.5] Line centi 1
[01:06.500] Line milli
[01:07:50] Line colon centi
[01:02:03.45] Line hour
"#;
        let lyrics = Lyrics::parse(lrc_data).expect("Should parse diverse timestamp formats");
        assert_eq!(lyrics.lines.len(), 4);
        assert!((lyrics.lines[0].time_seconds - 65.5).abs() < 1e-3);
        assert!((lyrics.lines[1].time_seconds - 66.5).abs() < 1e-3);
        assert!((lyrics.lines[2].time_seconds - 67.5).abs() < 1e-3);
        assert!((lyrics.lines[3].time_seconds - 3723.45).abs() < 1e-3);
    }

    #[test]
    fn test_karaoke_directory_sidecar_loading() {
        let temp_dir = std::env::temp_dir().join(format!("rusttracker_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let flac_path = temp_dir.join("Test Artist - Test Song (Karaoke).flac");
        let lrc_path = temp_dir.join("Test Artist - Test Song.lrc");
        let _ = fs::write(&flac_path, b"dummy audio content");
        let _ = fs::write(&lrc_path, "[00:10.00] Test line\n[00:20.00] Second line\n");

        let lyrics = load_lyrics_for_file(&flac_path.to_string_lossy());
        assert!(lyrics.is_some(), "Failed to load sidecar lyrics for temp file");
        let lyrics = lyrics.unwrap();
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].text, "Test line");

        let _ = fs::remove_file(&flac_path);
        let _ = fs::remove_file(&lrc_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_path_normalization() {
        assert_eq!(
            normalize_path_str("file:///home/naoki/Music/Cake%20-%20I%20Will%20Survive.flac"),
            "/home/naoki/Music/Cake - I Will Survive.flac"
        );
        assert_eq!(
            clean_song_stem("Cake_-_I_Will_Survive_(Instrumental)"),
            "cake i will survive"
        );
    }
}
