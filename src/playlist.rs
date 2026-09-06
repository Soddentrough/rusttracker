use std::path::{Path, PathBuf};

/// All supported media file extensions in RustTracker (lowercase, without leading dot)
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Tracker modules
    "mod", "xm", "s3m", "it", "stm", "669", "mtm", "med", "okt", "psm", "mptm",
    // Audio files
    "flac", "wav", "mp3", "ogg", "opus", "aac", "m4a", "mid", "midi", "aif", "aiff", "wma",
    // Video containers
    "mp4", "mkv", "webm", "avi", "mov",
];

/// Returns true if the file extension corresponds to a supported audio, video, or tracker format.
pub fn is_supported_media_extension(ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    SUPPORTED_EXTENSIONS.contains(&ext_lower.as_str())
}

/// Returns true if the file extension corresponds to a playlist file (.pls, .m3u, .m3u8).
pub fn is_playlist_extension(ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    matches!(ext_lower.as_str(), "pls" | "m3u" | "m3u8")
}

/// Recursively scans a directory for supported media files, returning them sorted alphabetically.
pub fn scan_directory(dir: &Path) -> Vec<String> {
    let mut results = Vec::new();
    collect_directory_files(dir, &mut results);
    results.sort();
    results
}

fn collect_directory_files(dir: &Path, results: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_directory_files(&path, results);
        } else if path.extension().and_then(|s| s.to_str()).is_some_and(is_supported_media_extension) {
            results.push(path.to_string_lossy().into_owned());
        }
    }
}

/// Parses a local playlist file (.pls, .m3u, .m3u8) and resolves any relative paths
/// against the playlist file's parent directory.
pub fn parse_playlist_file(playlist_path: &Path) -> Result<Vec<String>, std::io::Error> {
    let content = std::fs::read_to_string(playlist_path)?;
    let parent_dir = playlist_path.parent().unwrap_or_else(|| Path::new("."));
    let ext = playlist_path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    let mut paths = Vec::new();

    if ext == "pls" {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some((_, path_part)) = trimmed.strip_prefix("File").and_then(|s| s.split_once('=')) {
                let path_str = path_part.trim();
                if !path_str.is_empty() {
                    paths.push(resolve_path(path_str, parent_dir));
                }
            }
        }
    } else {
        // M3U / M3U8
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            paths.push(resolve_path(trimmed, parent_dir));
        }
    }

    Ok(paths)
}

fn resolve_path(entry: &str, base_dir: &Path) -> String {
    // If it's a URL or absolute path, preserve as is
    if entry.starts_with("http://") || entry.starts_with("https://") {
        return entry.to_string();
    }
    let p = PathBuf::from(entry);
    if p.is_absolute() {
        entry.to_string()
    } else {
        base_dir.join(p).to_string_lossy().into_owned()
    }
}

/// Expands a list of user input paths (CLI arguments, drag-and-drop, or IPC).
/// - Directories are recursively scanned for supported media formats.
/// - Playlist files (.m3u, .pls) are parsed and resolved into their constituent tracks.
/// - Normal files and stream URLs are passed through as-is.
pub fn expand_input_paths(paths: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();

    for path_str in paths {
        if path_str.is_empty() {
            continue;
        }

        // Keep remote URLs as-is
        if path_str.starts_with("http://") || path_str.starts_with("https://") {
            expanded.push(path_str.clone());
            continue;
        }

        let p = Path::new(path_str);
        if p.is_dir() {
            let scanned = scan_directory(p);
            expanded.extend(scanned);
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if is_playlist_extension(ext) {
                if let Ok(entries) = parse_playlist_file(p) {
                    expanded.extend(entries);
                } else {
                    expanded.push(path_str.clone());
                }
            } else {
                expanded.push(path_str.clone());
            }
        } else {
            expanded.push(path_str.clone());
        }
    }

    expanded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_is_supported_media_extension() {
        assert!(is_supported_media_extension("mod"));
        assert!(is_supported_media_extension("XM"));
        assert!(is_supported_media_extension("mp3"));
        assert!(is_supported_media_extension("FLAC"));
        assert!(is_supported_media_extension("mp4"));
        assert!(!is_supported_media_extension("txt"));
        assert!(!is_supported_media_extension("exe"));
    }

    #[test]
    fn test_parse_m3u() {
        let temp_dir = std::env::temp_dir().join("rusttracker_test_m3u");
        let _ = std::fs::create_dir_all(&temp_dir);
        let m3u_path = temp_dir.join("test.m3u");

        let content = "#EXTM3U\n#EXTINF:123,Test Song\ntrack1.mp3\n# Comment\n/abs/path/track2.flac\nhttp://example.com/stream\n";
        let mut file = File::create(&m3u_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let parsed = parse_playlist_file(&m3u_path).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(parsed[0].ends_with("track1.mp3"));
        assert_eq!(parsed[1], "/abs/path/track2.flac");
        assert_eq!(parsed[2], "http://example.com/stream");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_pls() {
        let temp_dir = std::env::temp_dir().join("rusttracker_test_pls");
        let _ = std::fs::create_dir_all(&temp_dir);
        let pls_path = temp_dir.join("test.pls");

        let content = "[playlist]\nNumberOfEntries=2\nFile1=song1.mod\nTitle1=Song 1\nFile2=https://example.com/live\nTitle2=Live\n";
        let mut file = File::create(&pls_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let parsed = parse_playlist_file(&pls_path).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].ends_with("song1.mod"));
        assert_eq!(parsed[1], "https://example.com/live");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
