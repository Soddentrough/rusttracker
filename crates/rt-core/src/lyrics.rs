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

        // Apply overall LRC [offset:+/-ms]: per standard LRC specification, a positive offset
        // shifts lyrics earlier (subtracts from timestamps) and a negative offset delays them.
        let offset_sec = (offset_ms as f64) / 1000.0;
        for line in &mut lines {
            line.time_seconds = (line.time_seconds - offset_sec).max(0.0);
        }

        // Sort lines chronologically
        lines.sort_by(|a, b| {
            a.time_seconds
                .partial_cmp(&b.time_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

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
            line.time_seconds
                .partial_cmp(&current_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(exact) => {
                // If there are multiple identical timestamps, return the last one
                let mut idx = exact;
                while idx + 1 < self.lines.len() && self.lines[idx + 1].time_seconds <= current_time
                {
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
                let (sec_str, frac_str) = p3_str
                    .split_once('.')
                    .or_else(|| p3_str.split_once(','))
                    .unwrap();
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

/// Normalizes a path string by stripping URI schemas (like `file://`), decoding UTF-8
/// percent-encoded byte sequences, and stripping leading `/` on Windows `file:///C:/...` URIs.
pub fn normalize_path_str(raw: &str) -> String {
    let mut s = raw.trim();
    let had_file_scheme = if s.len() >= 7 && s.as_bytes()[..7].eq_ignore_ascii_case(b"file://") {
        s = &s[7..];
        if s.len() >= 10
            && s.as_bytes()[..9].eq_ignore_ascii_case(b"localhost")
            && s.as_bytes()[9] == b'/'
        {
            s = &s[9..];
        }
        true
    } else {
        false
    };

    let bytes = s.as_bytes();
    let mut decoded_bytes = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(hex_str, 16)
        {
            decoded_bytes.push(val);
            i += 3;
            continue;
        }
        decoded_bytes.push(bytes[i]);
        i += 1;
    }

    let decoded = String::from_utf8_lossy(&decoded_bytes).into_owned();
    let db = decoded.as_bytes();
    if had_file_scheme
        && db.len() >= 3
        && db[0] == b'/'
        && db[1].is_ascii_alphabetic()
        && db[2] == b':'
        && (db.len() == 3 || db[3] == b'/' || db[3] == b'\\')
    {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

fn clean_song_stem(stem: &str) -> String {
    let mut s = stem.to_lowercase();
    for tag in &[
        "(instrumental)",
        "(vocals)",
        "(karaoke)",
        "(official)",
        "[flac]",
        "[mp3]",
        "(audio)",
        "(lyrics)",
    ] {
        s = s.replace(tag, " ");
    }
    let mapped: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Returns true iff `longer` begins with `shorter` followed immediately by a normalized word
/// delimiter (`' '`), preventing partial-token false positives such as `"track 1"` matching `"track 10"`.
fn stem_matches_at_delimiter_boundary(longer: &str, shorter: &str) -> bool {
    !shorter.is_empty()
        && longer.len() > shorter.len()
        && longer.starts_with(shorter)
        && longer.as_bytes().get(shorter.len()) == Some(&b' ')
}

/// Discovers an .lrc sidecar file for a given audio file path.
pub fn find_sidecar_lrc_path(audio_path: &str) -> Option<PathBuf> {
    let trimmed = audio_path.trim();
    if (trimmed.len() >= 7 && trimmed.as_bytes()[..7].eq_ignore_ascii_case(b"http://"))
        || (trimmed.len() >= 8 && trimmed.as_bytes()[..8].eq_ignore_ascii_case(b"https://"))
    {
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

        let mut prefix_fallback: Option<PathBuf> = None;

        for dir in &search_dirs {
            if let Ok(entries) = fs::read_dir(dir) {
                let mut dir_paths: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("lrc"))
                    })
                    .collect();
                dir_paths.sort();

                for entry_path in dir_paths {
                    if let Some(entry_stem) = entry_path.file_stem() {
                        let entry_stem_raw = entry_stem.to_string_lossy();
                        let entry_stem_clean = clean_song_stem(&entry_stem_raw);
                        if entry_stem_raw.eq_ignore_ascii_case(&stem_raw)
                            || (!stem_clean.is_empty() && entry_stem_clean == stem_clean)
                        {
                            return Some(entry_path);
                        }
                        if prefix_fallback.is_none()
                            && (stem_matches_at_delimiter_boundary(&stem_clean, &entry_stem_clean)
                                || stem_matches_at_delimiter_boundary(
                                    &entry_stem_clean,
                                    &stem_clean,
                                ))
                        {
                            prefix_fallback = Some(entry_path);
                        }
                    }
                }
            }
        }

        if prefix_fallback.is_some() {
            return prefix_fallback;
        }
    }

    None
}

/// Loads and parses an .lrc sidecar lyrics file for the specified audio path if present.
pub fn load_lyrics_for_file(audio_path: &str) -> Option<Lyrics> {
    let lrc_path = find_sidecar_lrc_path(audio_path)?;
    let content = fs::read_to_string(&lrc_path).ok()?;
    let mut lyrics = Lyrics::parse(&content)?;
    lyrics.file_name = lrc_path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned());
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
        assert_eq!(
            lyrics.lines[0].text,
            "I wanna take you somewhere so you know I care"
        );

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
        // Positive [offset:+500] shifts lyrics 500ms earlier (10.0s -> 9.5s) per LRC spec
        let lrc_pos = r#"
[offset:500]
[00:00.20] Clamped early line
[00:10.00] Offset line
"#;
        let lyrics_pos = Lyrics::parse(lrc_pos).expect("Should parse positive offset");
        assert_eq!(lyrics_pos.offset_ms, 500);
        assert!((lyrics_pos.lines[0].time_seconds - 0.0).abs() < 1e-4);
        assert!((lyrics_pos.lines[1].time_seconds - 9.5).abs() < 1e-4);

        // Negative [offset:-500] delays lyrics by 500ms (10.0s -> 10.5s)
        let lrc_neg = r#"
[offset:-500]
[00:10.00] Delayed line
"#;
        let lyrics_neg = Lyrics::parse(lrc_neg).expect("Should parse negative offset");
        assert_eq!(lyrics_neg.offset_ms, -500);
        assert!((lyrics_neg.lines[0].time_seconds - 10.5).abs() < 1e-4);
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
    fn test_karaoke_directory_sidecar_loading_and_stem_boundaries() {
        let temp_dir =
            std::env::temp_dir().join(format!("rusttracker_test_lrc_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Legitimate delimiter-bounded prefix match ("Test Artist - Test Song (Live).flac" -> "Test Artist - Test Song.lrc")
        let flac_path = temp_dir.join("Test Artist - Test Song (Live).flac");
        let lrc_path = temp_dir.join("Test Artist - Test Song.lrc");
        let _ = fs::write(&flac_path, b"dummy audio content");
        let _ = fs::write(&lrc_path, "[00:10.00] Test line\n[00:20.00] Second line\n");

        let lyrics = load_lyrics_for_file(&flac_path.to_string_lossy())
            .expect("Should load delimiter-bounded sidecar lyrics");
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].text, "Test line");

        // 2. Partial numeric/word prefix must NOT match ("Track 1.lrc" must NOT match "Track 10.flac")
        let track1_lrc = temp_dir.join("Track 1.lrc");
        let track10_flac = temp_dir.join("Track 10.flac");
        let _ = fs::write(&track1_lrc, "[00:05.00] Wrong track\n");
        let _ = fs::write(&track10_flac, b"dummy audio content");
        assert!(
            find_sidecar_lrc_path(&track10_flac.to_string_lossy()).is_none(),
            "Track 1.lrc must not match Track 10.flac"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_path_normalization() {
        assert_eq!(
            normalize_path_str("file:///home/naoki/Music/Cake%20-%20I%20Will%20Survive.flac"),
            "/home/naoki/Music/Cake - I Will Survive.flac"
        );
        // Multi-byte UTF-8 percent-encoded characters (Japanese + accented Latin)
        assert_eq!(
            normalize_path_str("file:///home/naoki/%E9%9F%B3%E6%A5%BD/Caf%C3%A9.flac"),
            "/home/naoki/音楽/Café.flac"
        );
        // Raw UTF-8 characters preserved without Latin-1 byte-to-char corruption
        assert_eq!(
            normalize_path_str("file:///home/naoki/音楽/曲%2001.flac"),
            "/home/naoki/音楽/曲 01.flac"
        );
        // Windows file:/// drive letter URI strips leading slash
        assert_eq!(
            normalize_path_str("file:///C:/Users/Naoki/Music/Song%2001.flac"),
            "C:/Users/Naoki/Music/Song 01.flac"
        );
        assert_eq!(
            normalize_path_str("FILE://localhost/D:/Music/Track.flac"),
            "D:/Music/Track.flac"
        );
        assert_eq!(
            normalize_path_str("file:///c%3A/Music/曲.flac"),
            "c:/Music/曲.flac"
        );
        // Regression: non-URI multi-byte UTF-8 paths where byte index 7 or 9 splits a codepoint
        assert_eq!(normalize_path_str("日本語.flac"), "日本語.flac");
        assert_eq!(
            normalize_path_str("file://a日本語/曲.mp3"),
            "a日本語/曲.mp3"
        );
        assert!(find_sidecar_lrc_path("HTTPS://example.com/日本語.flac").is_none());
        assert_eq!(
            clean_song_stem("Cake_-_I_Will_Survive_(Instrumental)"),
            "cake i will survive"
        );
    }
}
