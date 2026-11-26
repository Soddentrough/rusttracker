use std::path::{Path, PathBuf};

/// All supported media file extensions in RustTracker (lowercase, without leading dot)
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Tracker modules
    "mod", "xm", "s3m", "it", "stm", "669", "mtm", "med", "okt", "psm", "mptm",
    // Audio files
    "flac", "wav", "mp3", "ogg", "opus", "aac", "m4a", "mka", "mid", "midi", "aif", "aiff", "wma",
    "ac3", "eac3", "dts", "dtshd", "thd", "truehd", "mlp", "iamf", // Video containers
    "mp4", "m4v", "mkv", "webm", "avi", "mov", "ts", "m2ts", "mts", "flv", "wmv", "mpg", "mpeg",
    "ogv", "3gp", "vob", "aob", "iso",
];

/// Extensions selectable in file open dialogs (includes media files and playlists)
pub const FILE_DIALOG_EXTENSIONS: &[&str] = &[
    // Tracker modules
    "mod", "xm", "s3m", "it", "stm", "669", "mtm", "med", "okt", "psm", "mptm",
    // Audio files
    "flac", "wav", "mp3", "ogg", "opus", "aac", "m4a", "mka", "mid", "midi", "aif", "aiff", "wma",
    "ac3", "eac3", "dts", "dtshd", "thd", "truehd", "mlp", "iamf",
    // Video containers & disc images
    "mp4", "m4v", "mkv", "webm", "avi", "mov", "ts", "m2ts", "mts", "flv", "wmv", "mpg", "mpeg",
    "ogv", "3gp", "vob", "aob", "iso", // Playlist files
    "pls", "m3u", "m3u8",
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

/// Maximum directory recursion depth when expanding folders into playlists.
const MAX_DIRECTORY_SCAN_DEPTH: usize = 32;
const MAX_PLAYLIST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PLAYLIST_ENTRIES: usize = 50_000;
const MAX_PLAYLIST_NEST_DEPTH: usize = 4;

fn is_url_or_virtual_uri(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("file://")
        || lower.starts_with("mic://")
}

/// Recursively scans a directory for supported media files, returning them sorted alphabetically.
/// Guards against symlink cycles via canonicalized directory tracking and caps recursion depth.
pub fn scan_directory(dir: &Path) -> Vec<String> {
    let mut results = Vec::new();
    let mut visited = std::collections::HashSet::new();
    collect_directory_files(dir, 0, &mut visited, &mut results);
    results.sort();
    results
}

fn is_dvd_menu_or_continuation_vob(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let upper = name.to_uppercase();
    if upper == "VIDEO_TS.VOB" || upper == "AUDIO_TS.VOB" {
        return true;
    }
    if let Some(stem) = upper
        .strip_prefix("VTS_")
        .and_then(|s| s.strip_suffix(".VOB"))
        && let Some((vts_str, part_str)) = stem.split_once('_')
        && let (Ok(vts_num), Ok(part_num)) = (vts_str.parse::<u8>(), part_str.parse::<u8>())
    {
        if part_num == 0 {
            return true;
        }
        if part_num > 1
            && let Some(parent) = path.parent()
        {
            let part1 = parent.join(format!("VTS_{:02}_1.VOB", vts_num));
            let part1_lower = parent.join(format!("vts_{:02}_1.vob", vts_num));
            if part1.exists() || part1_lower.exists() {
                return true;
            }
        }
    }
    false
}

fn collect_directory_files(
    dir: &Path,
    depth: usize,
    visited: &mut std::collections::HashSet<PathBuf>,
    results: &mut Vec<String>,
) {
    if depth > MAX_DIRECTORY_SCAN_DEPTH {
        return;
    }
    let canonical = match std::fs::canonicalize(dir) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !visited.insert(canonical) {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if results.len() >= MAX_PLAYLIST_ENTRIES {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_directory_files(&path, depth + 1, visited, results);
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(is_supported_media_extension)
            && !is_dvd_menu_or_continuation_vob(&path)
        {
            results.push(path.to_string_lossy().into_owned());
        }
    }
}

/// Parses a local playlist file (.pls, .m3u, .m3u8) and resolves any relative paths
/// against the playlist file's parent directory.
pub fn parse_playlist_file(playlist_path: &Path) -> Result<Vec<String>, std::io::Error> {
    use std::io::Read;
    let file = std::fs::File::open(playlist_path)?;
    let mut raw = Vec::new();
    file.take(MAX_PLAYLIST_BYTES).read_to_end(&mut raw)?;
    let content = String::from_utf8_lossy(&raw);
    let content = content.trim_start_matches('\u{feff}');
    let parent_dir = playlist_path.parent().unwrap_or_else(|| Path::new("."));
    let ext = playlist_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut paths = Vec::new();

    if ext == "pls" {
        for line in content.lines() {
            if paths.len() >= MAX_PLAYLIST_ENTRIES {
                break;
            }
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with("file")
                && let Some((_, path_part)) = trimmed.split_once('=')
                && trimmed[4..trimmed.find('=').unwrap_or(4)]
                    .bytes()
                    .all(|b| b.is_ascii_digit())
            {
                let path_str = path_part.trim();
                if !path_str.is_empty() {
                    paths.push(resolve_path(path_str, parent_dir));
                }
            }
        }
    } else {
        // M3U / M3U8
        for line in content.lines() {
            if paths.len() >= MAX_PLAYLIST_ENTRIES {
                break;
            }
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
    if is_url_or_virtual_uri(entry) {
        return entry.to_string();
    }
    let p = Path::new(entry);
    // On Unix, a leading slash is absolute. On Windows, Path::is_absolute requires a drive letter
    // or UNC share, but a leading '/' or '\' is root-relative/absolute and should not be joined to base_dir.
    if p.is_absolute() || entry.starts_with('/') || entry.starts_with('\\') {
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
    let mut visited_playlists = std::collections::HashSet::new();
    let mut expanded = Vec::new();
    expand_input_paths_inner(paths, 0, &mut visited_playlists, &mut expanded);
    expanded
}

fn expand_input_paths_inner(
    paths: &[String],
    depth: usize,
    visited_playlists: &mut std::collections::HashSet<PathBuf>,
    expanded: &mut Vec<String>,
) {
    for path_str in paths {
        if expanded.len() >= MAX_PLAYLIST_ENTRIES {
            break;
        }
        if path_str.is_empty() {
            continue;
        }

        // Keep remote URLs / virtual streams as-is
        if is_url_or_virtual_uri(path_str) {
            expanded.push(path_str.clone());
            continue;
        }

        let p = Path::new(path_str);
        if p.is_dir() {
            let scanned = scan_directory(p);
            let remaining = MAX_PLAYLIST_ENTRIES.saturating_sub(expanded.len());
            expanded.extend(scanned.into_iter().take(remaining));
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if is_playlist_extension(ext) {
                let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
                if depth < MAX_PLAYLIST_NEST_DEPTH
                    && visited_playlists.insert(canonical)
                    && let Ok(entries) = parse_playlist_file(p)
                {
                    expand_input_paths_inner(&entries, depth + 1, visited_playlists, expanded);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempTestDir(PathBuf);

    impl TempTestDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "rt_playlist_test_{}_{}_{}",
                tag,
                std::process::id(),
                id
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp test dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_is_supported_media_extension() {
        assert!(is_supported_media_extension("mod"));
        assert!(is_supported_media_extension("XM"));
        assert!(is_supported_media_extension("mp3"));
        assert!(is_supported_media_extension("FLAC"));
        assert!(is_supported_media_extension("mp4"));
        assert!(is_supported_media_extension("iso"));
        assert!(is_supported_media_extension("VOB"));
        assert!(is_supported_media_extension("aob"));
        assert!(!is_supported_media_extension("txt"));
        assert!(!is_supported_media_extension("exe"));
        assert!(!is_supported_media_extension("dawproject"));
        assert!(!is_supported_media_extension("aaf"));
        assert!(!FILE_DIALOG_EXTENSIONS.contains(&"dawproject"));
        assert!(!FILE_DIALOG_EXTENSIONS.contains(&"aaf"));
    }

    #[test]
    fn test_parse_m3u() {
        let temp_dir = TempTestDir::new("m3u");
        let m3u_path = temp_dir.path().join("test.m3u");

        let content = "#EXTM3U\n#EXTINF:123,Test Song\ntrack1.mp3\n# Comment\n/abs/path/track2.flac\nhttp://example.com/stream\n";
        let mut file = File::create(&m3u_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let parsed = parse_playlist_file(&m3u_path).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(parsed[0].ends_with("track1.mp3"));
        assert_eq!(parsed[1], "/abs/path/track2.flac");
        assert_eq!(parsed[2], "http://example.com/stream");
    }

    #[test]
    fn test_parse_pls() {
        let temp_dir = TempTestDir::new("pls");
        let pls_path = temp_dir.path().join("test.pls");

        let content = "[playlist]\nNumberOfEntries=2\nFile1=song1.mod\nTitle1=Song 1\nFile2=https://example.com/live\nTitle2=Live\n";
        let mut file = File::create(&pls_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let parsed = parse_playlist_file(&pls_path).unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].ends_with("song1.mod"));
        assert_eq!(parsed[1], "https://example.com/live");
    }

    #[test]
    fn test_scan_directory_symlink_cycle_and_depth_limit() {
        let temp_dir = TempTestDir::new("scan_bounds");

        let root_track = temp_dir.path().join("root.flac");
        File::create(&root_track).unwrap();

        #[cfg(unix)]
        {
            let sub = temp_dir.path().join("sub");
            std::fs::create_dir_all(&sub).unwrap();
            File::create(sub.join("nested.mp3")).unwrap();
            // Create a self/parent symlink cycle: temp_dir/sub/cycle -> temp_dir
            std::os::unix::fs::symlink(temp_dir.path(), sub.join("cycle")).unwrap();
        }

        // Create a chain deeper than MAX_DIRECTORY_SCAN_DEPTH (32)
        let mut cur = temp_dir.path().to_path_buf();
        for depth in 0..=36 {
            cur = cur.join(format!("d{}", depth));
            std::fs::create_dir_all(&cur).unwrap();
            if depth == 31 {
                File::create(cur.join("within_limit.ogg")).unwrap();
            } else if depth == 35 {
                File::create(cur.join("beyond_limit.ogg")).unwrap();
            }
        }

        let found = scan_directory(temp_dir.path());
        assert!(found.iter().any(|p| p.ends_with("root.flac")));
        #[cfg(unix)]
        assert_eq!(
            found.iter().filter(|p| p.ends_with("nested.mp3")).count(),
            1
        );
        assert!(found.iter().any(|p| p.ends_with("within_limit.ogg")));
        assert!(
            !found.iter().any(|p| p.ends_with("beyond_limit.ogg")),
            "Files beyond MAX_DIRECTORY_SCAN_DEPTH should not be scanned"
        );
    }

    #[test]
    fn test_playlist_bom_and_pls_edge_cases() {
        let temp_dir = TempTestDir::new("bom_edges");

        let m3u8_path = temp_dir.path().join("utf8.m3u8");
        let m3u8_content = "\u{feff}#EXTM3U\n\n  # indented comment\nfile:///C:/Music/song.flac\nmic://default\nhttp://radio.example/live\n/abs/unix/track.wav\nrel/sub.mp3\n";
        std::fs::write(&m3u8_path, m3u8_content.as_bytes()).unwrap();
        let m3u8 = parse_playlist_file(&m3u8_path).unwrap();
        assert_eq!(m3u8.len(), 5);
        assert_eq!(m3u8[0], "file:///C:/Music/song.flac");
        assert_eq!(m3u8[1], "mic://default");
        assert_eq!(m3u8[2], "http://radio.example/live");
        assert_eq!(m3u8[3], "/abs/unix/track.wav");
        assert_eq!(
            m3u8[4],
            temp_dir.path().join("rel/sub.mp3").to_string_lossy()
        );

        let pls_path = temp_dir.path().join("mixed.pls");
        let pls_content = "\u{feff}[playlist]\nFILE1=first.ogg\nFileFoo=should_be_ignored.mp3\nfile2=  \nfile3=/direct/path.opus\n";
        std::fs::write(&pls_path, pls_content.as_bytes()).unwrap();
        let pls = parse_playlist_file(&pls_path).unwrap();
        assert_eq!(pls.len(), 2);
        assert_eq!(pls[0], temp_dir.path().join("first.ogg").to_string_lossy());
        assert_eq!(pls[1], "/direct/path.opus");
    }

    #[test]
    fn test_dvd_menu_and_continuation_vob_filtering() {
        let temp_dir = TempTestDir::new("dvd_vob_filter");

        let dvd_dir = temp_dir.path().join("with_part1");
        std::fs::create_dir_all(&dvd_dir).unwrap();
        for name in [
            "VIDEO_TS.VOB",
            "AUDIO_TS.VOB",
            "VTS_01_0.VOB",
            "VTS_01_1.VOB",
            "VTS_01_2.VOB",
        ] {
            File::create(dvd_dir.join(name)).unwrap();
        }
        let found_dvd = scan_directory(&dvd_dir);
        assert_eq!(
            found_dvd,
            vec![dvd_dir.join("VTS_01_1.VOB").to_string_lossy().into_owned()]
        );

        let standalone_dir = temp_dir.path().join("standalone_part2");
        std::fs::create_dir_all(&standalone_dir).unwrap();
        File::create(standalone_dir.join("VTS_01_2.VOB")).unwrap();
        let found_standalone = scan_directory(&standalone_dir);
        assert_eq!(
            found_standalone,
            vec![
                standalone_dir
                    .join("VTS_01_2.VOB")
                    .to_string_lossy()
                    .into_owned()
            ]
        );
    }

    #[test]
    fn test_expand_input_paths_nesting_cycles_and_urls() {
        let temp_dir = TempTestDir::new("expand_paths");

        let folder = temp_dir.path().join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let folder_track = folder.join("alpha.flac");
        File::create(&folder_track).unwrap();

        let pls_path = temp_dir.path().join("inner.pls");
        std::fs::write(&pls_path, "[playlist]\nFile1=nested.mod\n").unwrap();

        let outer_m3u = temp_dir.path().join("outer.m3u");
        std::fs::write(
            &outer_m3u,
            format!("{}\nhttps://stream.example/live\n", pls_path.display()),
        )
        .unwrap();

        // Mutually recursive playlists a.m3u <-> b.m3u must terminate without error/looping
        let a_m3u = temp_dir.path().join("a.m3u");
        let b_m3u = temp_dir.path().join("b.m3u");
        std::fs::write(&a_m3u, "b.m3u\ncycle_track.mp3\n").unwrap();
        std::fs::write(&b_m3u, "a.m3u\n").unwrap();

        // Depth limit (> MAX_PLAYLIST_NEST_DEPTH = 4) leaves deepest playlist path unexpanded
        let mut chain = Vec::new();
        for i in 0..=5 {
            chain.push(temp_dir.path().join(format!("nest_{i}.m3u")));
        }
        for i in 0..5 {
            std::fs::write(&chain[i], format!("{}\n", chain[i + 1].display())).unwrap();
        }
        std::fs::write(&chain[5], "deep_song.wav\n").unwrap();

        let expanded = expand_input_paths(&[
            String::new(),
            "mic://default".to_string(),
            folder.to_string_lossy().into_owned(),
            outer_m3u.to_string_lossy().into_owned(),
            a_m3u.to_string_lossy().into_owned(),
            chain[0].to_string_lossy().into_owned(),
        ]);

        assert!(expanded.contains(&"mic://default".to_string()));
        assert!(expanded.contains(&folder_track.to_string_lossy().into_owned()));
        assert!(
            expanded.contains(
                &temp_dir
                    .path()
                    .join("nested.mod")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert!(expanded.contains(&"https://stream.example/live".to_string()));
        assert!(
            expanded.contains(
                &temp_dir
                    .path()
                    .join("cycle_track.mp3")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert!(
            expanded.contains(&chain[4].to_string_lossy().into_owned()),
            "Expected nesting beyond MAX_PLAYLIST_NEST_DEPTH to stop expanding cleanly: {expanded:?}"
        );
    }
}
