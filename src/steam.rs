use std::error::Error;

/// Parses a binary Steam `shortcuts.vdf` buffer and returns all unsigned 32-bit `appid`s
/// for shortcut entries whose `AppName` or `Exe` references `rusttracker` (case-insensitive).
pub(crate) fn extract_rusttracker_shortcut_appids(vdf_bytes: &[u8]) -> Vec<u32> {
    let mut appids = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut current_appid: Option<u32> = None;
    let mut is_rusttracker = false;

    let read_cstr = |buf: &[u8], pos: &mut usize| -> Option<String> {
        let start = *pos;
        let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
        *pos = start + rel_end + 1;
        Some(String::from_utf8_lossy(&buf[start..start + rel_end]).into_owned())
    };

    while i < vdf_bytes.len() {
        let tag = vdf_bytes[i];
        i += 1;
        match tag {
            0x00 => {
                // Start of nested object
                if read_cstr(vdf_bytes, &mut i).is_none() {
                    break;
                }
                depth += 1;
                if depth == 2 {
                    current_appid = None;
                    is_rusttracker = false;
                }
            }
            0x01 => {
                // String property
                let Some(key) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                let Some(val) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                if depth == 2
                    && (key.eq_ignore_ascii_case("AppName") || key.eq_ignore_ascii_case("Exe"))
                    && val.to_ascii_lowercase().contains("rusttracker")
                {
                    is_rusttracker = true;
                }
            }
            0x02 => {
                // 32-bit integer property
                let Some(key) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                let Some(raw) = vdf_bytes.get(i..i + 4) else {
                    break;
                };
                i += 4;
                if depth == 2 && key.eq_ignore_ascii_case("appid") {
                    current_appid = Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]));
                }
            }
            0x03 => {
                // 32-bit float property
                if read_cstr(vdf_bytes, &mut i).is_none() || vdf_bytes.get(i..i + 4).is_none() {
                    break;
                }
                i += 4;
            }
            0x07 => {
                // 64-bit unsigned integer property
                if read_cstr(vdf_bytes, &mut i).is_none() || vdf_bytes.get(i..i + 8).is_none() {
                    break;
                }
                i += 8;
            }
            0x08 => {
                // End of nested object
                if depth == 2 {
                    if is_rusttracker
                        && let Some(id) = current_appid
                        && !appids.contains(&id)
                    {
                        appids.push(id);
                    }
                    current_appid = None;
                    is_rusttracker = false;
                }
                depth = depth.saturating_sub(1);
            }
            _ => break,
        }
    }
    appids
}

/// Normalizes any RustTracker shortcut entry in a binary `shortcuts.vdf` buffer so that
/// filename-based `AppName` values (e.g. `"RustTracker-SteamDeck-GamePad.AppImage"`) become
/// `"RustTracker"` and an empty `icon` field is populated with `default_icon_path`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn normalize_rusttracker_shortcuts_vdf(
    vdf_bytes: &[u8],
    default_icon_path: Option<&str>,
) -> Option<Vec<u8>> {
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut is_rusttracker = false;
    let mut app_name_span: Option<(usize, usize, String)> = None;
    let mut icon_span: Option<(usize, usize, String)> = None;

    let read_cstr_span = |buf: &[u8], pos: &mut usize| -> Option<(usize, usize, String)> {
        let start = *pos;
        let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
        let end = start + rel_end;
        *pos = end + 1;
        Some((
            start,
            end,
            String::from_utf8_lossy(&buf[start..end]).into_owned(),
        ))
    };

    while i < vdf_bytes.len() {
        let tag = vdf_bytes[i];
        i += 1;
        match tag {
            0x00 => {
                if read_cstr_span(vdf_bytes, &mut i).is_none() {
                    break;
                }
                depth += 1;
                if depth == 2 {
                    is_rusttracker = false;
                    app_name_span = None;
                    icon_span = None;
                }
            }
            0x01 => {
                let Some((_, _, key)) = read_cstr_span(vdf_bytes, &mut i) else {
                    break;
                };
                let Some((val_start, val_end, val)) = read_cstr_span(vdf_bytes, &mut i) else {
                    break;
                };
                if depth == 2 {
                    if (key.eq_ignore_ascii_case("AppName") || key.eq_ignore_ascii_case("Exe"))
                        && val.to_ascii_lowercase().contains("rusttracker")
                    {
                        is_rusttracker = true;
                    }
                    if key.eq_ignore_ascii_case("AppName") {
                        app_name_span = Some((val_start, val_end, val));
                    } else if key.eq_ignore_ascii_case("icon") {
                        icon_span = Some((val_start, val_end, val));
                    }
                }
            }
            0x02 | 0x03 => {
                if read_cstr_span(vdf_bytes, &mut i).is_none() {
                    break;
                };
                if vdf_bytes.get(i..i + 4).is_none() {
                    break;
                }
                i += 4;
            }
            0x07 => {
                if read_cstr_span(vdf_bytes, &mut i).is_none() {
                    break;
                };
                if vdf_bytes.get(i..i + 8).is_none() {
                    break;
                }
                i += 8;
            }
            0x08 => {
                if depth == 2 {
                    if is_rusttracker {
                        if let Some((start, end, name)) = app_name_span.take() {
                            let lower = name.to_ascii_lowercase();
                            if name != "RustTracker"
                                && (lower.ends_with(".appimage")
                                    || lower.starts_with("rusttracker-")
                                    || lower.starts_with("rusttracker_")
                                    || lower == "rusttracker")
                            {
                                replacements.push((start, end, "RustTracker".to_string()));
                            }
                        }
                        if let Some((start, end, icon)) = icon_span.take()
                            && icon.is_empty()
                            && let Some(icon_path) = default_icon_path
                            && !icon_path.is_empty()
                        {
                            replacements.push((start, end, icon_path.to_string()));
                        }
                    }
                    is_rusttracker = false;
                    app_name_span = None;
                    icon_span = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => break,
        }
    }

    if replacements.is_empty() {
        return None;
    }

    replacements.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut out = vdf_bytes.to_vec();
    for (start, end, new_val) in replacements {
        out.splice(start..end, new_val.into_bytes());
    }
    Some(out)
}

/// Inserts or updates a RustTracker shortcut entry in a binary `shortcuts.vdf` buffer,
/// returning the updated buffer and the shortcut's unsigned 32-bit `appid`.
pub(crate) fn upsert_rusttracker_shortcut_in_vdf(
    vdf_bytes: &[u8],
    exe_path: &str,
    start_dir: &str,
    icon_path: &str,
) -> (Vec<u8>, u32) {
    let existing_ids = extract_rusttracker_shortcut_appids(vdf_bytes);
    if let Some(&appid) = existing_ids.first() {
        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        let mut i = 0usize;
        let mut depth = 0usize;
        let mut is_rusttracker = false;
        let mut app_name_span: Option<(usize, usize)> = None;
        let mut exe_span: Option<(usize, usize)> = None;
        let mut start_dir_span: Option<(usize, usize)> = None;
        let mut icon_span: Option<(usize, usize)> = None;

        let read_cstr_span = |buf: &[u8], pos: &mut usize| -> Option<(usize, usize, String)> {
            let start = *pos;
            let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
            let end = start + rel_end;
            *pos = end + 1;
            Some((
                start,
                end,
                String::from_utf8_lossy(&buf[start..end]).into_owned(),
            ))
        };

        while i < vdf_bytes.len() {
            let tag = vdf_bytes[i];
            i += 1;
            match tag {
                0x00 => {
                    if read_cstr_span(vdf_bytes, &mut i).is_none() {
                        break;
                    }
                    depth += 1;
                    if depth == 2 {
                        is_rusttracker = false;
                        app_name_span = None;
                        exe_span = None;
                        start_dir_span = None;
                        icon_span = None;
                    }
                }
                0x01 => {
                    let Some((_, _, key)) = read_cstr_span(vdf_bytes, &mut i) else {
                        break;
                    };
                    let Some((val_start, val_end, val)) = read_cstr_span(vdf_bytes, &mut i) else {
                        break;
                    };
                    if depth == 2 {
                        if (key.eq_ignore_ascii_case("AppName") || key.eq_ignore_ascii_case("Exe"))
                            && val.to_ascii_lowercase().contains("rusttracker")
                        {
                            is_rusttracker = true;
                        }
                        if key.eq_ignore_ascii_case("AppName") {
                            app_name_span = Some((val_start, val_end));
                        } else if key.eq_ignore_ascii_case("Exe") {
                            exe_span = Some((val_start, val_end));
                        } else if key.eq_ignore_ascii_case("StartDir") {
                            start_dir_span = Some((val_start, val_end));
                        } else if key.eq_ignore_ascii_case("icon") {
                            icon_span = Some((val_start, val_end));
                        }
                    }
                }
                0x02 | 0x03 => {
                    if read_cstr_span(vdf_bytes, &mut i).is_none() {
                        break;
                    }
                    if vdf_bytes.get(i..i + 4).is_none() {
                        break;
                    }
                    i += 4;
                }
                0x07 => {
                    if read_cstr_span(vdf_bytes, &mut i).is_none() {
                        break;
                    }
                    if vdf_bytes.get(i..i + 8).is_none() {
                        break;
                    }
                    i += 8;
                }
                0x08 => {
                    if depth == 2 {
                        if is_rusttracker {
                            if let Some((s, e)) = app_name_span.take() {
                                replacements.push((s, e, "RustTracker".to_string()));
                            }
                            if !exe_path.is_empty()
                                && let Some((s, e)) = exe_span.take()
                            {
                                replacements.push((s, e, format!("\"{exe_path}\"")));
                            }
                            if !start_dir.is_empty()
                                && let Some((s, e)) = start_dir_span.take()
                            {
                                replacements.push((s, e, start_dir.to_string()));
                            }
                            if !icon_path.is_empty()
                                && let Some((s, e)) = icon_span.take()
                            {
                                replacements.push((s, e, icon_path.to_string()));
                            }
                        }
                        is_rusttracker = false;
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => break,
            }
        }
        replacements.sort_by_key(|a| std::cmp::Reverse(a.0));
        let mut out = vdf_bytes.to_vec();
        for (start, end, new_val) in replacements {
            out.splice(start..end, new_val.into_bytes());
        }
        return (out, appid);
    }

    // Walk existing binary VDF stream to find the next shortcut index and exact byte offset
    // of the closing `0x08` of the root `shortcuts` map (depth 1 -> 0).
    let appid: u32 = 3929032336; // 0xEA304690
    let mut next_idx = 0usize;
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut root_close_offset: Option<usize> = None;
    let read_cstr = |buf: &[u8], pos: &mut usize| -> Option<String> {
        let start = *pos;
        let rel_end = buf.get(start..)?.iter().position(|&b| b == 0)?;
        *pos = start + rel_end + 1;
        Some(String::from_utf8_lossy(&buf[start..start + rel_end]).into_owned())
    };
    while i < vdf_bytes.len() {
        let tag = vdf_bytes[i];
        i += 1;
        match tag {
            0x00 => {
                let Some(key) = read_cstr(vdf_bytes, &mut i) else {
                    break;
                };
                depth += 1;
                if depth == 2
                    && let Ok(n) = key.parse::<usize>()
                {
                    next_idx = next_idx.max(n + 1);
                }
            }
            0x01 => {
                if read_cstr(vdf_bytes, &mut i).is_none() || read_cstr(vdf_bytes, &mut i).is_none()
                {
                    break;
                }
            }
            0x02 | 0x03 => {
                if read_cstr(vdf_bytes, &mut i).is_none() || vdf_bytes.get(i..i + 4).is_none() {
                    break;
                }
                i += 4;
            }
            0x07 => {
                if read_cstr(vdf_bytes, &mut i).is_none() || vdf_bytes.get(i..i + 8).is_none() {
                    break;
                }
                i += 8;
            }
            0x08 => {
                if depth == 1 {
                    root_close_offset = Some(i - 1);
                }
                depth = depth.saturating_sub(1);
            }
            _ => break,
        }
    }

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);

    let mut entry = Vec::new();
    entry.push(0x00);
    entry.extend_from_slice(next_idx.to_string().as_bytes());
    entry.push(0x00);
    entry.extend_from_slice(b"\x02appid\x00");
    entry.extend_from_slice(&appid.to_le_bytes());
    entry.extend_from_slice(b"\x01AppName\x00RustTracker\x00");
    entry.extend_from_slice(b"\x01Exe\x00\"");
    entry.extend_from_slice(exe_path.as_bytes());
    entry.extend_from_slice(b"\"\x00");
    entry.extend_from_slice(b"\x01StartDir\x00");
    entry.extend_from_slice(start_dir.as_bytes());
    entry.push(0x00);
    entry.extend_from_slice(b"\x01icon\x00");
    entry.extend_from_slice(icon_path.as_bytes());
    entry.push(0x00);
    entry.extend_from_slice(b"\x01ShortcutPath\x00\x00");
    entry.extend_from_slice(b"\x01LaunchOptions\x00\x00");
    entry.extend_from_slice(b"\x02IsHidden\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x02AllowDesktopConfig\x00\x01\x00\x00\x00");
    entry.extend_from_slice(b"\x02AllowOverlay\x00\x01\x00\x00\x00");
    entry.extend_from_slice(b"\x02OpenVR\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x02Devkit\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x01DevkitGameID\x00\x00");
    entry.extend_from_slice(b"\x02DevkitOverrideAppID\x00\x00\x00\x00\x00");
    entry.extend_from_slice(b"\x02LastPlayTime\x00");
    entry.extend_from_slice(&now_epoch.to_le_bytes());
    entry.extend_from_slice(b"\x01FlatpakAppID\x00\x00");
    entry.extend_from_slice(b"\x01sortas\x00\x00");
    entry.extend_from_slice(b"\x00tags\x00\x08\x08");

    let out = if vdf_bytes.starts_with(b"\x00shortcuts\x00")
        && let Some(close_pos) = root_close_offset
    {
        let mut buf = vdf_bytes.to_vec();
        buf.splice(close_pos..close_pos, entry);
        buf
    } else {
        let mut buf = b"\x00shortcuts\x00".to_vec();
        buf.extend_from_slice(&entry);
        buf.extend_from_slice(b"\x08\x08");
        buf
    };
    (out, appid)
}

pub(crate) fn register_steam_shortcut_cli() -> Result<(), Box<dyn Error>> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Err("HOME environment variable is not set".into());
    };
    let exe_path = std::env::var("APPIMAGE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "/home/deck/Downloads/RustTracker-SteamDeck.AppImage".to_string());
    let start_dir = std::path::Path::new(&exe_path)
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| format!("{}/", s.trim_end_matches('/')))
        .unwrap_or_else(|| "/home/deck/Downloads/".to_string());
    let icon_path = home.join(".local/share/icons/hicolor/512x512/apps/rusttracker.png");
    let icon_path_str = icon_path.to_string_lossy().into_owned();

    // Stop steam-launcher.service if running on SteamOS so Steam flushes shortcuts.vdf first
    let steam_service_was_active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "steam-launcher.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if steam_service_was_active {
        println!("[steam] Stopping steam-launcher.service to safely update shortcuts.vdf...");
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "stop", "steam-launcher.service"])
            .status();
    }

    let mut total_registered = 0usize;
    for rel in [".local/share/Steam", ".steam/steam"] {
        let root = home.join(rel);
        let userdata = root.join("userdata");
        if !userdata.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&userdata) {
            for entry in entries.flatten() {
                let user_dir = entry.path();
                if !user_dir.is_dir() {
                    continue;
                }
                let config_dir = user_dir.join("config");
                let _ = std::fs::create_dir_all(&config_dir);
                let vdf_path = config_dir.join("shortcuts.vdf");
                let existing = std::fs::read(&vdf_path).unwrap_or_default();
                let (updated, appid) = upsert_rusttracker_shortcut_in_vdf(
                    &existing,
                    &exe_path,
                    &start_dir,
                    &icon_path_str,
                );
                let tmp_path = config_dir.join(format!("shortcuts.vdf.tmp.{}", std::process::id()));
                {
                    use std::io::Write;
                    let mut f = std::fs::File::create(&tmp_path)?;
                    f.write_all(&updated)?;
                    f.sync_all()?;
                }
                std::fs::rename(&tmp_path, &vdf_path)?;
                println!(
                    "[steam] Registered 'RustTracker' (AppID {}) in {}",
                    appid,
                    vdf_path.display()
                );
                total_registered += 1;
            }
        }
        if total_registered > 0 {
            break;
        }
    }

    auto_provision_steam_artwork();

    if steam_service_was_active {
        println!("[steam] Restarting steam-launcher.service...");
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "start", "steam-launcher.service"])
            .status();
    }

    println!(
        "[steam] Successfully registered RustTracker and provisioned SteamOS Home Screen artwork ({} user profile(s)).",
        total_registered
    );
    Ok(())
}

/// Copies Steam Grid (`userdata/<id>/config/grid/`) and LibraryCache (`appcache/librarycache/`)
/// artwork for all discovered RustTracker non-Steam shortcuts under `steam_root`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn provision_steam_shortcut_artwork_in_root(
    steam_root: &std::path::Path,
    steamgrid_dir: &std::path::Path,
    icon_png: Option<&std::path::Path>,
) -> usize {
    let userdata_dir = steam_root.join("userdata");
    let Ok(entries) = std::fs::read_dir(&userdata_dir) else {
        return 0;
    };
    let copy_if_missing = |src: &std::path::Path, dst: &std::path::Path| -> bool {
        if !dst.exists() && src.exists() {
            std::fs::copy(src, dst).is_ok()
        } else {
            false
        }
    };

    let header_src = steamgrid_dir.join("header.jpg");
    let capsule_src = steamgrid_dir.join("library_600x900.jpg");
    let hero_src = steamgrid_dir.join("library_hero.jpg");
    let hero_blur_src = steamgrid_dir.join("library_hero_blur.jpg");
    let logo_src = steamgrid_dir.join("logo.png");
    let icon_jpg_src = steamgrid_dir.join("icon.jpg");

    let libcache_dir = steam_root.join("appcache").join("librarycache");
    let mut provisioned_shortcuts = 0usize;

    for entry in entries.flatten() {
        let user_dir = entry.path();
        if !user_dir.is_dir() {
            continue;
        }
        let vdf_path = user_dir.join("config").join("shortcuts.vdf");
        let Ok(vdf_bytes) = std::fs::read(&vdf_path) else {
            continue;
        };
        let appids = extract_rusttracker_shortcut_appids(&vdf_bytes);
        if appids.is_empty() {
            continue;
        }
        let icon_path_str = icon_png.and_then(|p| p.to_str());
        if let Some(updated_vdf) = normalize_rusttracker_shortcuts_vdf(&vdf_bytes, icon_path_str) {
            let _ = std::fs::write(&vdf_path, updated_vdf);
        }
        let grid_dir = user_dir.join("config").join("grid");
        let _ = std::fs::create_dir_all(&grid_dir);
        let _ = std::fs::create_dir_all(&libcache_dir);

        for appid in appids {
            provisioned_shortcuts += 1;
            let _ = copy_if_missing(&header_src, &grid_dir.join(format!("{appid}.jpg")));
            let _ = copy_if_missing(&capsule_src, &grid_dir.join(format!("{appid}p.jpg")));
            let _ = copy_if_missing(&hero_src, &grid_dir.join(format!("{appid}_hero.jpg")));
            let _ = copy_if_missing(&logo_src, &grid_dir.join(format!("{appid}_logo.png")));
            let _ = copy_if_missing(&icon_jpg_src, &grid_dir.join(format!("{appid}_icon.jpg")));
            if let Some(png) = icon_png {
                let _ = copy_if_missing(png, &grid_dir.join(format!("{appid}_icon.png")));
            }

            let app_cache_sub = libcache_dir.join(appid.to_string());
            let _ = std::fs::create_dir_all(&app_cache_sub);
            let _ = copy_if_missing(&header_src, &app_cache_sub.join("header.jpg"));
            let _ = copy_if_missing(&header_src, &app_cache_sub.join("library_header.jpg"));
            let _ = copy_if_missing(&capsule_src, &app_cache_sub.join("library_600x900.jpg"));
            let _ = copy_if_missing(&hero_src, &app_cache_sub.join("library_hero.jpg"));
            let _ = copy_if_missing(&hero_blur_src, &app_cache_sub.join("library_hero_blur.jpg"));
            let _ = copy_if_missing(&logo_src, &app_cache_sub.join("logo.png"));
            let _ = copy_if_missing(&icon_jpg_src, &app_cache_sub.join("icon.jpg"));

            let _ = copy_if_missing(
                &header_src,
                &libcache_dir.join(format!("{appid}_header.jpg")),
            );
            let _ = copy_if_missing(
                &header_src,
                &libcache_dir.join(format!("{appid}_library_header.jpg")),
            );
            let _ = copy_if_missing(
                &capsule_src,
                &libcache_dir.join(format!("{appid}_library_600x900.jpg")),
            );
            let _ = copy_if_missing(
                &hero_src,
                &libcache_dir.join(format!("{appid}_library_hero.jpg")),
            );
            let _ = copy_if_missing(
                &hero_blur_src,
                &libcache_dir.join(format!("{appid}_library_hero_blur.jpg")),
            );
            let _ = copy_if_missing(&logo_src, &libcache_dir.join(format!("{appid}_logo.png")));
            let _ = copy_if_missing(
                &icon_jpg_src,
                &libcache_dir.join(format!("{appid}_icon.jpg")),
            );
        }
    }
    provisioned_shortcuts
}

pub(crate) fn auto_provision_steam_artwork() {
    #[cfg(target_os = "linux")]
    {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
            return;
        };
        let mut steamgrid_candidates = Vec::new();
        let mut icon_candidates = Vec::new();
        if let Some(appdir) = std::env::var_os("RUSTTRACKER_APPDIR")
            .or_else(|| std::env::var_os("APPDIR"))
            .map(std::path::PathBuf::from)
        {
            steamgrid_candidates.push(appdir.join("usr/share/rusttracker/steamgrid"));
            icon_candidates
                .push(appdir.join("usr/share/icons/hicolor/512x512/apps/rusttracker.png"));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            steamgrid_candidates.push(exe_dir.join("../share/rusttracker/steamgrid"));
            steamgrid_candidates.push(exe_dir.join("assets/steamgrid"));
            icon_candidates
                .push(exe_dir.join("../share/icons/hicolor/512x512/apps/rusttracker.png"));
            icon_candidates.push(exe_dir.join("assets/icons/hicolor/512x512/apps/rusttracker.png"));
        }
        steamgrid_candidates.push(std::path::PathBuf::from("assets/steamgrid"));
        icon_candidates.push(std::path::PathBuf::from(
            "assets/icons/hicolor/512x512/apps/rusttracker.png",
        ));

        let Some(steamgrid_dir) = steamgrid_candidates
            .into_iter()
            .find(|p| p.join("header.jpg").exists())
        else {
            return;
        };
        let bundled_icon_png = icon_candidates.into_iter().find(|p| p.exists());
        let persistent_icon_path =
            home.join(".local/share/icons/hicolor/512x512/apps/rusttracker.png");
        if !persistent_icon_path.exists()
            && let Some(ref src_icon) = bundled_icon_png
        {
            if let Some(parent) = persistent_icon_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(src_icon, &persistent_icon_path);
        }
        let icon_png = if persistent_icon_path.exists() {
            Some(persistent_icon_path)
        } else {
            bundled_icon_png
        };

        for rel in [".local/share/Steam", ".steam/steam"] {
            let root = home.join(rel);
            if root.join("userdata").is_dir() {
                let _ = provision_steam_shortcut_artwork_in_root(
                    &root,
                    &steamgrid_dir,
                    icon_png.as_deref(),
                );
            }
        }
    }
}
