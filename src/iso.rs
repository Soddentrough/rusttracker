use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const DVD_SECTOR_SIZE: u64 = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoExtent {
    pub file_path: PathBuf,
    pub physical_offset: u64,
    pub byte_len: u64,
    pub logical_start: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DvdCellMapping {
    pub cell_index: usize,
    pub start_time_sec: f64,
    pub duration_sec: f64,
    pub logical_byte_start: u64,
    pub logical_byte_len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfoAudioStreamInfo {
    pub index: usize,
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub bit_depth: Option<u8>,
    pub language: Option<String>,
    pub commentary_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IsoMediaKind {
    DvdVideo,
    DvdAudio,
    DataFile { inner_name: String, ext: String },
}

#[derive(Clone, Debug)]
pub struct ResolvedIsoMedia {
    pub volume_label: Option<String>,
    pub media_kind: IsoMediaKind,
    pub extents: Vec<IsoExtent>,
    pub total_bytes: u64,
    pub ifo_duration_sec: Option<f64>,
    pub cells: Vec<DvdCellMapping>,
    pub audio_streams: Vec<IfoAudioStreamInfo>,
}

impl ResolvedIsoMedia {
    /// Maps a target playback timestamp (in seconds) to a 2048-byte sector-aligned
    /// logical byte offset within the concatenated virtual stream.
    pub fn time_to_logical_byte(&self, target_sec: f64, fallback_duration_sec: f64) -> u64 {
        if self.total_bytes == 0 {
            return 0;
        }
        let target = target_sec.max(0.0);

        if !self.cells.is_empty() {
            for cell in &self.cells {
                let cell_end = cell.start_time_sec + cell.duration_sec;
                if target <= cell_end || cell.cell_index + 1 == self.cells.len() {
                    let frac = if cell.duration_sec > 0.001 {
                        ((target - cell.start_time_sec) / cell.duration_sec).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let offset_in_cell = (frac * cell.logical_byte_len as f64) as u64;
                    let raw_byte = (cell.logical_byte_start + offset_in_cell)
                        .min(self.total_bytes.saturating_sub(DVD_SECTOR_SIZE));
                    return (raw_byte / DVD_SECTOR_SIZE) * DVD_SECTOR_SIZE;
                }
            }
        }

        let total_dur = self
            .ifo_duration_sec
            .unwrap_or(fallback_duration_sec)
            .max(0.001);
        let frac = (target / total_dur).clamp(0.0, 1.0);
        let raw_byte = ((frac * self.total_bytes as f64) as u64)
            .min(self.total_bytes.saturating_sub(DVD_SECTOR_SIZE));
        (raw_byte / DVD_SECTOR_SIZE) * DVD_SECTOR_SIZE
    }
}

#[derive(Clone, Debug)]
struct IsoFileEntry {
    path: String,
    extents: Vec<(u64, u64)>, // (physical_byte_offset, byte_len)
    total_len: u64,
}

#[derive(Clone, Debug)]
pub struct RawDvdCell {
    pub duration_sec: f64,
    pub first_sector: u64,
    pub last_sector: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedVtsIfo {
    pub duration_sec: f64,
    pub cells: Vec<RawDvdCell>,
    pub audio_streams: Vec<IfoAudioStreamInfo>,
    pub vob_start_sector_rel: u64,
    pub vts_last_sector_rel: u64,
    pub ifo_last_sector_rel: u64,
}

#[inline]
fn bcd_to_u8(byte: u8) -> u8 {
    ((byte >> 4) * 10) + (byte & 0x0F)
}

/// Decodes a 4-byte DVD BCD playback time (`HH:MM:SS:FF` with frame-rate flags in bits 7..6 of byte 3).
pub fn parse_dvd_bcd_time(bytes: &[u8]) -> f64 {
    if bytes.len() < 4 {
        return 0.0;
    }
    let hours = bcd_to_u8(bytes[0]) as f64;
    let minutes = bcd_to_u8(bytes[1]) as f64;
    let seconds = bcd_to_u8(bytes[2]) as f64;
    let frames = bcd_to_u8(bytes[3] & 0x3F) as f64;
    let fps = match (bytes[3] >> 6) & 0x03 {
        1 => 25.0,
        3 => 29.97,
        _ => 29.97,
    };
    hours * 3600.0 + minutes * 60.0 + seconds + (frames / fps)
}

/// Parses a `VTS_xx_0.IFO` (`DVDVIDEO-VTS`) binary buffer to extract the longest Program Chain's
/// duration, cell playback sector map (`C_PBIT`), and audio stream attributes (`VTS_AST_ATRT`).
pub fn parse_vts_ifo_bytes(ifo: &[u8]) -> Option<ParsedVtsIfo> {
    if ifo.len() < 2048 || &ifo[0..12] != b"DVDVIDEO-VTS" {
        return None;
    }

    let vts_last_sector_rel =
        u32::from_be_bytes([ifo[0x0C], ifo[0x0D], ifo[0x0E], ifo[0x0F]]) as u64;
    let ifo_last_sector_rel =
        u32::from_be_bytes([ifo[0x1C], ifo[0x1D], ifo[0x1E], ifo[0x1F]]) as u64;
    let vob_start_sector_rel =
        u32::from_be_bytes([ifo[0xC4], ifo[0xC5], ifo[0xC6], ifo[0xC7]]) as u64;
    let pgci_sector = u32::from_be_bytes([ifo[0xCC], ifo[0xCD], ifo[0xCE], ifo[0xCF]]) as usize;

    // 1. Parse VTS_AST_ATRT (Audio Stream Attribute Table at 0x0202)
    let num_audio = u16::from_be_bytes([ifo[0x0202], ifo[0x0203]]) as usize;
    let mut audio_streams = Vec::new();
    for i in 0..num_audio.min(8) {
        let base = 0x0204 + i * 8;
        if base + 8 > ifo.len() {
            break;
        }
        let attr = &ifo[base..base + 8];
        let coding_mode = (attr[0] >> 5) & 0x07;
        let codec = match coding_mode {
            0 => "AC3".to_string(),
            2 | 3 => "MP2".to_string(),
            4 => "LPCM".to_string(),
            6 => "DTS".to_string(),
            _ => "Audio".to_string(),
        };
        let lang_type = (attr[0] >> 2) & 0x03;
        let language =
            if lang_type == 1 && attr[2].is_ascii_alphabetic() && attr[3].is_ascii_alphabetic() {
                Some(
                    String::from_utf8_lossy(&attr[2..4])
                        .to_lowercase()
                        .trim()
                        .to_string(),
                )
            } else {
                None
            };
        let quant = (attr[1] >> 6) & 0x03;
        let bit_depth = if coding_mode == 4 {
            match quant {
                0 => Some(16),
                1 => Some(20),
                2 => Some(24),
                _ => None,
            }
        } else {
            None
        };
        let sample_rate = match (attr[1] >> 4) & 0x03 {
            1 => 96000,
            _ => 48000,
        };
        let channels = ((attr[1] & 0x07) + 1) as u16;
        let commentary_label = match attr[5] {
            2 => Some("Visually Impaired".to_string()),
            3 => Some("Director's Commentary".to_string()),
            4 => Some("Alternate Commentary".to_string()),
            _ => None,
        };

        audio_streams.push(IfoAudioStreamInfo {
            index: i,
            codec,
            channels,
            sample_rate,
            bit_depth,
            language,
            commentary_label,
        });
    }

    // 2. Parse VTS_PGCI (Program Chain Information Table)
    let mut best_duration = 0.0f64;
    let mut best_cells = Vec::new();

    if pgci_sector > 0 {
        let pgci_base = pgci_sector * (DVD_SECTOR_SIZE as usize);
        if pgci_base + 8 <= ifo.len() {
            let num_pgcs = u16::from_be_bytes([ifo[pgci_base], ifo[pgci_base + 1]]) as usize;
            for p in 0..num_pgcs.min(256) {
                let ptr_off = pgci_base + 8 + p * 8;
                if ptr_off + 8 > ifo.len() {
                    break;
                }
                let pgc_rel = u32::from_be_bytes([
                    ifo[ptr_off + 4],
                    ifo[ptr_off + 5],
                    ifo[ptr_off + 6],
                    ifo[ptr_off + 7],
                ]) as usize;
                let pgc_base = pgci_base.saturating_add(pgc_rel);
                if pgc_base + 0xEC > ifo.len() {
                    continue;
                }

                let num_cells = ifo[pgc_base + 3] as usize;
                let pgc_dur = parse_dvd_bcd_time(&ifo[pgc_base + 4..pgc_base + 8]);
                let c_pbit_off =
                    u16::from_be_bytes([ifo[pgc_base + 0xE8], ifo[pgc_base + 0xE9]]) as usize;

                let mut cells = Vec::new();
                if c_pbit_off > 0 && num_cells > 0 {
                    let c_pbit_base = pgc_base + c_pbit_off;
                    for c in 0..num_cells {
                        let cell_off = c_pbit_base + c * 24;
                        if cell_off + 24 > ifo.len() {
                            break;
                        }
                        let cell = &ifo[cell_off..cell_off + 24];
                        let block_mode = (cell[0] >> 6) & 0x03;
                        let block_type = (cell[0] >> 4) & 0x03;
                        // Skip secondary angle blocks (keep normal cells and Angle 1)
                        if block_type == 1 && block_mode > 1 {
                            continue;
                        }
                        let cell_dur = parse_dvd_bcd_time(&cell[4..8]);
                        let first_sector =
                            u32::from_be_bytes([cell[8], cell[9], cell[10], cell[11]]) as u64;
                        let last_sector =
                            u32::from_be_bytes([cell[20], cell[21], cell[22], cell[23]]) as u64;
                        if last_sector >= first_sector {
                            cells.push(RawDvdCell {
                                duration_sec: cell_dur,
                                first_sector,
                                last_sector,
                            });
                        }
                    }
                }

                let cells_dur_sum: f64 = cells.iter().map(|c| c.duration_sec).sum();
                let effective_dur = pgc_dur.max(cells_dur_sum);
                if effective_dur > best_duration && !cells.is_empty() {
                    best_duration = effective_dur;
                    best_cells = cells;
                } else if best_cells.is_empty() && pgc_dur > best_duration {
                    best_duration = pgc_dur;
                }
            }
        }
    }

    Some(ParsedVtsIfo {
        duration_sec: best_duration,
        cells: best_cells,
        audio_streams,
        vob_start_sector_rel,
        vts_last_sector_rel,
        ifo_last_sector_rel,
    })
}

/// Parses the ISO 9660 filesystem inside an `.ISO` image and returns `(volume_label, files)`.
fn parse_iso9660_entries(
    file: &mut File,
    file_len: u64,
) -> Option<(Option<String>, Vec<IsoFileEntry>)> {
    if file_len < 17 * DVD_SECTOR_SIZE {
        return None;
    }

    let mut volume_label = None;
    let mut root_lba = 0u64;
    let mut root_len = 0u64;
    let mut found_pvd = false;
    let mut sector_buf = [0u8; 2048];

    for sec in 16..32 {
        let offset = sec * DVD_SECTOR_SIZE;
        if offset + DVD_SECTOR_SIZE > file_len {
            break;
        }
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut sector_buf).ok()?;

        if &sector_buf[1..6] != b"CD001" {
            if sec == 16 {
                return None;
            }
            break;
        }

        let vd_type = sector_buf[0];
        if vd_type == 1 {
            // Primary Volume Descriptor
            let raw_label = String::from_utf8_lossy(&sector_buf[40..72])
                .trim()
                .trim_matches('\0')
                .to_string();
            if !raw_label.is_empty() {
                volume_label = Some(raw_label);
            }
            let root_rec = &sector_buf[156..190];
            root_lba =
                u32::from_le_bytes([root_rec[2], root_rec[3], root_rec[4], root_rec[5]]) as u64;
            root_len =
                u32::from_le_bytes([root_rec[10], root_rec[11], root_rec[12], root_rec[13]]) as u64;
            found_pvd = true;
            break;
        } else if vd_type == 255 {
            break;
        }
    }

    if !found_pvd || root_lba == 0 || root_len == 0 {
        return None;
    }

    let mut files = Vec::new();
    let mut visited_dirs = std::collections::HashSet::new();
    visited_dirs.insert(root_lba);
    read_iso9660_dir_recursive(
        file,
        file_len,
        root_lba,
        root_len,
        "",
        0,
        &mut visited_dirs,
        &mut files,
    );

    Some((volume_label, files))
}

#[allow(clippy::too_many_arguments)]
fn read_iso9660_dir_recursive(
    file: &mut File,
    file_len: u64,
    dir_lba: u64,
    dir_len: u64,
    prefix: &str,
    depth: usize,
    visited_dirs: &mut std::collections::HashSet<u64>,
    out_files: &mut Vec<IsoFileEntry>,
) {
    if depth > 8 || dir_len == 0 || dir_len > 16 * 1024 * 1024 {
        return;
    }
    let dir_offset = dir_lba * DVD_SECTOR_SIZE;
    if dir_offset.saturating_add(dir_len) > file_len {
        return;
    }

    let mut dir_buf = vec![0u8; dir_len as usize];
    if file.seek(SeekFrom::Start(dir_offset)).is_err() || file.read_exact(&mut dir_buf).is_err() {
        return;
    }

    let mut subdirs: Vec<(u64, u64, String)> = Vec::new();
    let mut pos = 0usize;

    while pos < dir_buf.len() {
        let rec_len = dir_buf[pos] as usize;
        if rec_len == 0 {
            // Directory records do not cross 2048-byte sector boundaries; advance to next sector
            pos = ((pos / 2048) + 1) * 2048;
            continue;
        }
        if pos + rec_len > dir_buf.len() || rec_len < 34 {
            break;
        }

        let rec = &dir_buf[pos..pos + rec_len];
        let ext_attr_len = rec[1] as u64;
        let extent_lba = u32::from_le_bytes([rec[2], rec[3], rec[4], rec[5]]) as u64 + ext_attr_len;
        let data_len = u32::from_le_bytes([rec[10], rec[11], rec[12], rec[13]]) as u64;
        let flags = rec[25];
        let is_dir = (flags & 0x02) != 0;
        let name_len = rec[32] as usize;

        if 33 + name_len <= rec_len && name_len > 0 {
            let raw_name = &rec[33..33 + name_len];
            if raw_name != [0] && raw_name != [1] {
                let mut name = String::from_utf8_lossy(raw_name).to_string();
                if let Some((base, _ver)) = name.split_once(';') {
                    name = base.to_string();
                }
                if name.ends_with('.') {
                    name.pop();
                }
                let full_path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };

                if is_dir {
                    if extent_lba > 0 && visited_dirs.insert(extent_lba) {
                        subdirs.push((extent_lba, data_len, full_path));
                    }
                } else {
                    let phys_off = extent_lba * DVD_SECTOR_SIZE;
                    if phys_off.saturating_add(data_len) <= file_len && data_len > 0 {
                        // Support ISO 9660 Level 3 multi-extent files (> 4 GB)
                        if let Some(last) = out_files.last_mut()
                            && last.path.eq_ignore_ascii_case(&full_path)
                        {
                            last.extents.push((phys_off, data_len));
                            last.total_len += data_len;
                        } else {
                            out_files.push(IsoFileEntry {
                                path: full_path,
                                extents: vec![(phys_off, data_len)],
                                total_len: data_len,
                            });
                        }
                    }
                }
            }
        }

        pos += rec_len;
    }

    for (sub_lba, sub_len, sub_prefix) in subdirs {
        read_iso9660_dir_recursive(
            file,
            file_len,
            sub_lba,
            sub_len,
            &sub_prefix,
            depth + 1,
            visited_dirs,
            out_files,
        );
    }
}

/// Scans the first 32 MB of an ISO image at 2048-byte sector boundaries for `DVDVIDEO-VMG` or
/// `DVDVIDEO-VTS` headers. Used as a fallback for UDF-only DVD images where the ISO 9660 bridge
/// directory tree is omitted.
fn scan_udf_dvd_video_sectors(
    file: &mut File,
    file_len: u64,
    iso_path: &Path,
) -> Option<ResolvedIsoMedia> {
    let max_scan_bytes = file_len.min(32 * 1024 * 1024);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut vmg_sector: Option<u64> = None;
    let mut vts_sectors: Vec<u64> = Vec::new();

    let mut offset = 16 * DVD_SECTOR_SIZE;
    while offset < max_scan_bytes {
        let to_read = ((max_scan_bytes - offset) as usize).min(chunk.len());
        if file.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }
        let Ok(n) = file.read(&mut chunk[..to_read]) else {
            break;
        };
        if n < 2048 {
            break;
        }
        let mut s = 0usize;
        while s + 12 <= n {
            let magic = &chunk[s..s + 12];
            let sec_lba = (offset + s as u64) / DVD_SECTOR_SIZE;
            if magic == b"DVDVIDEO-VMG" && vmg_sector.is_none() {
                vmg_sector = Some(sec_lba);
            } else if magic == b"DVDVIDEO-VTS" && !vts_sectors.contains(&sec_lba) {
                vts_sectors.push(sec_lba);
            }
            s += 2048;
        }
        offset += n as u64;
    }

    // If we found VIDEO_TS.IFO (`DVDVIDEO-VMG`), read VMG_TT_SRPT to discover all VTS_xx_0.IFO sectors
    if let Some(vmg_lba) = vmg_sector {
        let mut vmg_hdr = [0u8; 2048];
        if file
            .seek(SeekFrom::Start(vmg_lba * DVD_SECTOR_SIZE))
            .is_ok()
            && file.read_exact(&mut vmg_hdr).is_ok()
        {
            let tt_srpt_rel =
                u32::from_be_bytes([vmg_hdr[0xC4], vmg_hdr[0xC5], vmg_hdr[0xC6], vmg_hdr[0xC7]])
                    as u64;
            if tt_srpt_rel > 0 {
                let tt_srpt_off = (vmg_lba + tt_srpt_rel) * DVD_SECTOR_SIZE;
                let mut srpt_buf = [0u8; 2048];
                if tt_srpt_off + 2048 <= file_len
                    && file.seek(SeekFrom::Start(tt_srpt_off)).is_ok()
                    && file.read_exact(&mut srpt_buf).is_ok()
                {
                    let num_titles = u16::from_be_bytes([srpt_buf[0], srpt_buf[1]]) as usize;
                    for t in 0..num_titles.min(99) {
                        let entry_off = 8 + t * 12;
                        if entry_off + 12 > srpt_buf.len() {
                            break;
                        }
                        let vts_lba = u32::from_be_bytes([
                            srpt_buf[entry_off + 8],
                            srpt_buf[entry_off + 9],
                            srpt_buf[entry_off + 10],
                            srpt_buf[entry_off + 11],
                        ]) as u64;
                        if vts_lba > 0 && !vts_sectors.contains(&vts_lba) {
                            vts_sectors.push(vts_lba);
                        }
                    }
                }
            }
        }
    }

    if vts_sectors.is_empty() {
        return None;
    }

    type UdfVtsCandidate = (f64, u64, ParsedVtsIfo, Vec<(u64, u64)>);
    let mut best_candidate: Option<UdfVtsCandidate> = None;

    for vts_lba in vts_sectors {
        let ifo_off = vts_lba * DVD_SECTOR_SIZE;
        if ifo_off + 2048 > file_len {
            continue;
        }
        let mut hdr = [0u8; 2048];
        if file.seek(SeekFrom::Start(ifo_off)).is_err() || file.read_exact(&mut hdr).is_err() {
            continue;
        }
        if &hdr[0..12] != b"DVDVIDEO-VTS" {
            continue;
        }
        let ifo_last_rel = u32::from_be_bytes([hdr[0x1C], hdr[0x1D], hdr[0x1E], hdr[0x1F]]) as u64;
        let ifo_len = ((ifo_last_rel + 1) * DVD_SECTOR_SIZE).min(4 * 1024 * 1024);
        if ifo_off + ifo_len > file_len {
            continue;
        }
        let mut ifo_bytes = vec![0u8; ifo_len as usize];
        if file.seek(SeekFrom::Start(ifo_off)).is_err() || file.read_exact(&mut ifo_bytes).is_err()
        {
            continue;
        }
        let Some(parsed) = parse_vts_ifo_bytes(&ifo_bytes) else {
            continue;
        };
        if parsed.vob_start_sector_rel == 0
            || parsed.vts_last_sector_rel <= parsed.vob_start_sector_rel
        {
            continue;
        }
        let vob_start_lba = vts_lba + parsed.vob_start_sector_rel;
        // Exclude the trailing VTS_xx_0.BUP backup sectors at the end of the VTS
        let bup_sectors = parsed.ifo_last_sector_rel + 1;
        let vob_end_lba = (vts_lba + parsed.vts_last_sector_rel + 1).saturating_sub(bup_sectors);
        if vob_end_lba <= vob_start_lba {
            continue;
        }
        let vob_phys_off = vob_start_lba * DVD_SECTOR_SIZE;
        let vob_byte_len = ((vob_end_lba - vob_start_lba) * DVD_SECTOR_SIZE)
            .min(file_len.saturating_sub(vob_phys_off));
        if vob_byte_len == 0 {
            continue;
        }

        let is_better = match &best_candidate {
            None => true,
            Some((best_dur, best_len, _, _)) => {
                if parsed.duration_sec > *best_dur + 1.0 {
                    true
                } else if (parsed.duration_sec - *best_dur).abs() <= 1.0 {
                    vob_byte_len > *best_len
                } else {
                    false
                }
            }
        };
        if is_better {
            best_candidate = Some((
                parsed.duration_sec,
                vob_byte_len,
                parsed,
                vec![(vob_phys_off, vob_byte_len)],
            ));
        }
    }

    let (_, _, parsed_ifo, raw_vob_extents) = best_candidate?;
    Some(build_resolved_dvd_media(
        iso_path,
        None,
        IsoMediaKind::DvdVideo,
        &raw_vob_extents,
        Some(parsed_ifo),
    ))
}

fn build_resolved_dvd_media(
    iso_path: &Path,
    volume_label: Option<String>,
    media_kind: IsoMediaKind,
    raw_extents: &[(u64, u64)],
    parsed_ifo: Option<ParsedVtsIfo>,
) -> ResolvedIsoMedia {
    let with_paths: Vec<(PathBuf, u64, u64)> = raw_extents
        .iter()
        .map(|&(off, len)| (iso_path.to_path_buf(), off, len))
        .collect();
    build_resolved_dvd_media_with_paths(iso_path, volume_label, media_kind, &with_paths, parsed_ifo)
}

fn build_resolved_dvd_media_with_paths(
    iso_path: &Path,
    volume_label: Option<String>,
    media_kind: IsoMediaKind,
    vob_file_extents: &[(PathBuf, u64, u64)],
    parsed_ifo: Option<ParsedVtsIfo>,
) -> ResolvedIsoMedia {
    let total_vob_space: u64 = vob_file_extents.iter().map(|(_, _, len)| *len).sum();

    let slice_vob_range = |vob_rel_start: u64,
                           vob_rel_len: u64,
                           out_extents: &mut Vec<IsoExtent>,
                           logical_cursor: &mut u64| {
        let range_end = vob_rel_start.saturating_add(vob_rel_len);
        let mut vob_pos = 0u64;
        for (f_path, phys_off, f_len) in vob_file_extents {
            let f_start = vob_pos;
            let f_end = vob_pos + *f_len;
            vob_pos = f_end;

            let overlap_start = vob_rel_start.max(f_start);
            let overlap_end = range_end.min(f_end);
            if overlap_end > overlap_start {
                let seg_phys = phys_off + (overlap_start - f_start);
                let seg_len = overlap_end - overlap_start;

                if let Some(last) = out_extents.last_mut()
                    && last.file_path == *f_path
                    && last.physical_offset + last.byte_len == seg_phys
                {
                    last.byte_len += seg_len;
                } else {
                    out_extents.push(IsoExtent {
                        file_path: f_path.clone(),
                        physical_offset: seg_phys,
                        byte_len: seg_len,
                        logical_start: *logical_cursor,
                    });
                }
                *logical_cursor += seg_len;
            }
        }
    };

    if let Some(ifo) = parsed_ifo {
        let mut extents = Vec::new();
        let mut cells = Vec::new();
        let mut logical_cursor = 0u64;
        let mut cumulative_time = 0.0f64;

        for (idx, raw_cell) in ifo.cells.iter().enumerate() {
            let cell_vob_start = raw_cell.first_sector * DVD_SECTOR_SIZE;
            let cell_vob_len = (raw_cell.last_sector - raw_cell.first_sector + 1) * DVD_SECTOR_SIZE;
            if cell_vob_start < total_vob_space {
                let clamped_len = cell_vob_len.min(total_vob_space - cell_vob_start);
                let cell_logical_start = logical_cursor;
                slice_vob_range(
                    cell_vob_start,
                    clamped_len,
                    &mut extents,
                    &mut logical_cursor,
                );
                let actual_cell_bytes = logical_cursor - cell_logical_start;
                if actual_cell_bytes > 0 {
                    cells.push(DvdCellMapping {
                        cell_index: idx,
                        start_time_sec: cumulative_time,
                        duration_sec: raw_cell.duration_sec,
                        logical_byte_start: cell_logical_start,
                        logical_byte_len: actual_cell_bytes,
                    });
                    cumulative_time += raw_cell.duration_sec;
                }
            }
        }

        if !extents.is_empty() && logical_cursor >= DVD_SECTOR_SIZE {
            let dur = if ifo.duration_sec > 0.0 {
                Some(ifo.duration_sec)
            } else if cumulative_time > 0.0 {
                Some(cumulative_time)
            } else {
                None
            };
            return ResolvedIsoMedia {
                volume_label,
                media_kind,
                extents,
                total_bytes: logical_cursor,
                ifo_duration_sec: dur,
                cells,
                audio_streams: ifo.audio_streams,
            };
        }

        let mut full_extents = Vec::new();
        let mut full_cursor = 0u64;
        slice_vob_range(0, total_vob_space, &mut full_extents, &mut full_cursor);
        return ResolvedIsoMedia {
            volume_label,
            media_kind,
            extents: full_extents,
            total_bytes: full_cursor,
            ifo_duration_sec: if ifo.duration_sec > 0.0 {
                Some(ifo.duration_sec)
            } else {
                None
            },
            cells: Vec::new(),
            audio_streams: ifo.audio_streams,
        };
    }

    let mut full_extents = Vec::new();
    let mut full_cursor = 0u64;
    if vob_file_extents.is_empty() {
        let len = std::fs::metadata(iso_path).map(|m| m.len()).unwrap_or(0);
        full_extents.push(IsoExtent {
            file_path: iso_path.to_path_buf(),
            physical_offset: 0,
            byte_len: len,
            logical_start: 0,
        });
        full_cursor = len;
    } else {
        slice_vob_range(0, total_vob_space, &mut full_extents, &mut full_cursor);
    }

    ResolvedIsoMedia {
        volume_label,
        media_kind,
        extents: full_extents,
        total_bytes: full_cursor,
        ifo_duration_sec: None,
        cells: Vec::new(),
        audio_streams: Vec::new(),
    }
}

/// Extracts `(vts_num, part_num)` from a filename like `"VTS_01_1.VOB"` or `"ATS_02_3.AOB"`.
fn parse_vts_vob_filename(filename: &str, prefix: &str, ext: &str) -> Option<(u8, u8)> {
    let upper = filename.to_uppercase();
    let stem = upper.strip_prefix(prefix)?.strip_suffix(ext)?;
    let (vts_str, part_str) = stem.split_once('_')?;
    let vts_num = vts_str.parse::<u8>().ok()?;
    let part_num = part_str.parse::<u8>().ok()?;
    Some((vts_num, part_num))
}

/// Resolves an `.ISO` disc image (or a `VTS_xx_1.VOB` / `ATS_xx_1.AOB` file in a `VIDEO_TS` / `AUDIO_TS` folder)
/// into its constituent byte extents, `.IFO` Program Chain duration, cell seek table, and audio stream descriptors.
pub fn resolve_iso_or_vob_media(path: &Path) -> Result<Option<ResolvedIsoMedia>> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext == "vob" || ext == "aob" {
        return Ok(resolve_disk_vob_chain(path));
    }

    if ext != "iso" {
        return Ok(None);
    }

    let mut file =
        File::open(path).with_context(|| format!("Failed to open ISO: {}", path.display()))?;
    let file_len = file.metadata()?.len();

    if let Some((volume_label, entries)) = parse_iso9660_entries(&mut file, file_len) {
        // 1. Check for DVD-Video (`VIDEO_TS/VTS_xx_1..9.VOB`)
        let mut vts_vob_groups: std::collections::BTreeMap<u8, Vec<(u8, &IsoFileEntry)>> =
            std::collections::BTreeMap::new();
        let mut vts_ifo_map: std::collections::BTreeMap<u8, &IsoFileEntry> =
            std::collections::BTreeMap::new();

        for entry in &entries {
            let file_name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
            if let Some((vts_num, part_num)) = parse_vts_vob_filename(file_name, "VTS_", ".VOB") {
                if part_num >= 1 {
                    vts_vob_groups
                        .entry(vts_num)
                        .or_default()
                        .push((part_num, entry));
                }
            } else if let Some((vts_num, 0)) = parse_vts_vob_filename(file_name, "VTS_", ".IFO") {
                vts_ifo_map.insert(vts_num, entry);
            }
        }

        if !vts_vob_groups.is_empty() {
            type IsoVtsCandidate = (f64, u64, Option<ParsedVtsIfo>, Vec<(u64, u64)>);
            let mut best_vts: Option<IsoVtsCandidate> = None;

            for (vts_num, mut parts) in vts_vob_groups {
                parts.sort_by_key(|(part_num, _)| *part_num);
                let mut raw_extents = Vec::new();
                let mut vts_bytes = 0u64;
                for (_, entry) in parts {
                    for &(off, len) in &entry.extents {
                        raw_extents.push((off, len));
                        vts_bytes += len;
                    }
                }

                let parsed_ifo = vts_ifo_map.get(&vts_num).and_then(|ifo_entry| {
                    let (off, len) = *ifo_entry.extents.first()?;
                    let read_len = (len as usize).min(4 * 1024 * 1024);
                    let mut buf = vec![0u8; read_len];
                    file.seek(SeekFrom::Start(off)).ok()?;
                    file.read_exact(&mut buf).ok()?;
                    parse_vts_ifo_bytes(&buf)
                });

                let dur = parsed_ifo.as_ref().map(|p| p.duration_sec).unwrap_or(0.0);
                let is_better = match &best_vts {
                    None => true,
                    Some((best_dur, best_bytes, _, _)) => {
                        if dur > *best_dur + 1.0 {
                            true
                        } else if (dur - *best_dur).abs() <= 1.0 {
                            vts_bytes > *best_bytes
                        } else {
                            false
                        }
                    }
                };
                if is_better {
                    best_vts = Some((dur, vts_bytes, parsed_ifo, raw_extents));
                }
            }

            if let Some((_, _, parsed_ifo, raw_extents)) = best_vts {
                return Ok(Some(build_resolved_dvd_media(
                    path,
                    volume_label,
                    IsoMediaKind::DvdVideo,
                    &raw_extents,
                    parsed_ifo,
                )));
            }
        }

        // 2. Check for DVD-Audio (`AUDIO_TS/ATS_xx_1..9.AOB`)
        let mut ats_aob_groups: std::collections::BTreeMap<u8, Vec<(u8, &IsoFileEntry)>> =
            std::collections::BTreeMap::new();
        for entry in &entries {
            let file_name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
            if let Some((ats_num, part_num)) = parse_vts_vob_filename(file_name, "ATS_", ".AOB")
                && part_num >= 1
            {
                ats_aob_groups
                    .entry(ats_num)
                    .or_default()
                    .push((part_num, entry));
            }
        }
        if let Some((_, mut parts)) = ats_aob_groups
            .into_iter()
            .max_by_key(|(_, parts)| parts.iter().map(|(_, e)| e.total_len).sum::<u64>())
        {
            parts.sort_by_key(|(part_num, _)| *part_num);
            let mut raw_extents = Vec::new();
            for (_, entry) in parts {
                for &(off, len) in &entry.extents {
                    raw_extents.push((off, len));
                }
            }
            return Ok(Some(build_resolved_dvd_media(
                path,
                volume_label,
                IsoMediaKind::DvdAudio,
                &raw_extents,
                None,
            )));
        }

        // 3. Check for Data ISO containing supported media files (.flac, .mp3, .mkv, .mp4, .mod, .xm, .s3m, .it, .vob, etc.)
        let mut media_entries: Vec<(&IsoFileEntry, String)> = entries
            .iter()
            .filter_map(|e| {
                let inner_ext = Path::new(&e.path)
                    .extension()
                    .and_then(|s| s.to_str())?
                    .to_lowercase();
                if inner_ext != "iso" && crate::playlist::is_supported_media_extension(&inner_ext) {
                    Some((e, inner_ext))
                } else {
                    None
                }
            })
            .collect();

        if !media_entries.is_empty() {
            media_entries.sort_by_key(|(a, _)| std::cmp::Reverse(a.total_len));
            let (chosen, inner_ext) = &media_entries[0];
            let mut extents = Vec::new();
            let mut cursor = 0u64;
            for &(off, len) in &chosen.extents {
                extents.push(IsoExtent {
                    file_path: path.to_path_buf(),
                    physical_offset: off,
                    byte_len: len,
                    logical_start: cursor,
                });
                cursor += len;
            }
            return Ok(Some(ResolvedIsoMedia {
                volume_label,
                media_kind: IsoMediaKind::DataFile {
                    inner_name: chosen.path.clone(),
                    ext: inner_ext.clone(),
                },
                extents,
                total_bytes: cursor,
                ifo_duration_sec: None,
                cells: Vec::new(),
                audio_streams: Vec::new(),
            }));
        }
    }

    // 4. Fallback: Scan for UDF-only DVD-Video (`DVDVIDEO-VMG` / `DVDVIDEO-VTS`) sector headers
    if let Some(resolved_udf) = scan_udf_dvd_video_sectors(&mut file, file_len, path) {
        return Ok(Some(resolved_udf));
    }

    Ok(None)
}

/// If the user opens `VTS_xx_1.VOB` on disk inside a `VIDEO_TS` folder, automatically chains
/// sibling `VTS_xx_2.VOB .. VTS_xx_9.VOB` segments and parses `VTS_xx_0.IFO` when present.
fn resolve_disk_vob_chain(vob_path: &Path) -> Option<ResolvedIsoMedia> {
    let file_name = vob_path.file_name()?.to_str()?;
    let parent_dir = vob_path.parent()?;

    if let Some((vts_num, part_num)) = parse_vts_vob_filename(file_name, "VTS_", ".VOB")
        && part_num >= 1
    {
        let mut vob_files: Vec<(u8, PathBuf, u64)> = Vec::new();
        if part_num == 1
            && let Ok(dir_entries) = std::fs::read_dir(parent_dir)
        {
            for entry in dir_entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str())
                    && let Some((v, pt)) = parse_vts_vob_filename(name, "VTS_", ".VOB")
                    && v == vts_num
                    && pt >= 1
                    && let Ok(meta) = p.metadata()
                    && meta.len() > 0
                {
                    vob_files.push((pt, p, meta.len()));
                }
            }
        }
        if vob_files.is_empty() {
            let len = vob_path.metadata().ok()?.len();
            vob_files.push((part_num, vob_path.to_path_buf(), len));
        }
        vob_files.sort_by_key(|(pt, _, _)| *pt);

        let parsed_ifo = if part_num == 1 {
            std::fs::read_dir(parent_dir).ok().and_then(|entries| {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if let Some(name) = p.file_name().and_then(|s| s.to_str())
                        && parse_vts_vob_filename(name, "VTS_", ".IFO") == Some((vts_num, 0))
                        && let Ok(bytes) = std::fs::read(&p)
                    {
                        return parse_vts_ifo_bytes(&bytes);
                    }
                }
                None
            })
        } else {
            None
        };

        let extents_with_paths: Vec<(PathBuf, u64, u64)> = vob_files
            .into_iter()
            .map(|(_, p, len)| (p, 0, len))
            .collect();

        return Some(build_resolved_dvd_media_with_paths(
            vob_path,
            None,
            IsoMediaKind::DvdVideo,
            &extents_with_paths,
            parsed_ifo,
        ));
    }

    None
}

/// Seekable, zero-copy virtual stream over one or more `IsoExtent` byte ranges.
pub struct IsoVirtualStream {
    extents: Vec<IsoExtent>,
    total_len: u64,
    pos: u64,
    active_file: Option<(PathBuf, File, u64)>,
}

impl IsoVirtualStream {
    pub fn new(extents: Vec<IsoExtent>) -> Result<Self> {
        let total_len = extents.iter().map(|e| e.byte_len).sum();
        let mut s = Self {
            extents,
            total_len,
            pos: 0,
            active_file: None,
        };
        if let Some(first) = s.extents.first() {
            let mut f = File::open(&first.file_path)?;
            f.seek(SeekFrom::Start(first.physical_offset))?;
            s.active_file = Some((first.file_path.clone(), f, first.physical_offset));
        }
        Ok(s)
    }

    pub fn total_len(&self) -> u64 {
        self.total_len
    }

    fn find_extent_index(&self, logical_pos: u64) -> Option<usize> {
        self.extents
            .binary_search_by(|ext| {
                if logical_pos < ext.logical_start {
                    std::cmp::Ordering::Greater
                } else if logical_pos >= ext.logical_start + ext.byte_len {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()
    }
}

impl Read for IsoVirtualStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.pos >= self.total_len {
            return Ok(0);
        }

        let Some(ext_idx) = self.find_extent_index(self.pos) else {
            return Ok(0);
        };
        let ext = self.extents[ext_idx].clone();
        let offset_in_ext = self.pos - ext.logical_start;
        let target_phys = ext.physical_offset + offset_in_ext;
        let max_read = (ext.byte_len - offset_in_ext).min(buf.len() as u64) as usize;

        let need_reopen = self
            .active_file
            .as_ref()
            .is_none_or(|(p, _, _)| *p != ext.file_path);
        if need_reopen {
            let mut f = File::open(&ext.file_path)?;
            f.seek(SeekFrom::Start(target_phys))?;
            self.active_file = Some((ext.file_path.clone(), f, target_phys));
        }

        let (_, file, cur_phys) = self.active_file.as_mut().unwrap();
        if *cur_phys != target_phys {
            file.seek(SeekFrom::Start(target_phys))?;
            *cur_phys = target_phys;
        }

        let n = file.read(&mut buf[..max_read])?;
        *cur_phys += n as u64;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for IsoVirtualStream {
    fn seek(&mut self, style: SeekFrom) -> std::io::Result<u64> {
        let new_pos: i128 = match style {
            SeekFrom::Start(off) => off as i128,
            SeekFrom::Current(rel) => self.pos as i128 + rel as i128,
            SeekFrom::End(rel) => self.total_len as i128 + rel as i128,
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid seek before start of ISO virtual stream",
            ));
        }
        self.pos = (new_pos as u64).min(self.total_len);
        Ok(self.pos)
    }
}

impl symphonia::core::io::MediaSource for IsoVirtualStream {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.total_len)
    }
}

pub struct CustomAvioGuard {
    avio_ctx: *mut ffmpeg_next::ffi::AVIOContext,
    opaque_ptr: *mut IsoVirtualStream,
}

unsafe impl Send for CustomAvioGuard {}

impl Drop for CustomAvioGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.avio_ctx.is_null() {
                if !(*self.avio_ctx).buffer.is_null() {
                    ffmpeg_next::ffi::av_freep(
                        &mut (*self.avio_ctx).buffer as *mut *mut u8 as *mut std::ffi::c_void,
                    );
                }
                ffmpeg_next::ffi::avio_context_free(&mut self.avio_ctx);
            }
            if !self.opaque_ptr.is_null() {
                drop(Box::from_raw(self.opaque_ptr));
                self.opaque_ptr = std::ptr::null_mut();
            }
        }
    }
}

unsafe extern "C" fn avio_read_packet(
    opaque: *mut std::ffi::c_void,
    buf: *mut u8,
    buf_size: std::ffi::c_int,
) -> std::ffi::c_int {
    if opaque.is_null() || buf.is_null() || buf_size <= 0 {
        return ffmpeg_next::ffi::AVERROR_EOF;
    }
    let stream = unsafe { &mut *(opaque as *mut IsoVirtualStream) };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf, buf_size as usize) };
    match stream.read(slice) {
        Ok(0) => ffmpeg_next::ffi::AVERROR_EOF,
        Ok(n) => n as std::ffi::c_int,
        Err(_) => ffmpeg_next::ffi::AVERROR_EOF,
    }
}

unsafe extern "C" fn avio_seek_packet(
    opaque: *mut std::ffi::c_void,
    offset: i64,
    whence: std::ffi::c_int,
) -> i64 {
    if opaque.is_null() {
        return -1;
    }
    let stream = unsafe { &mut *(opaque as *mut IsoVirtualStream) };
    if (whence & ffmpeg_next::ffi::AVSEEK_SIZE) != 0 {
        return stream.total_len() as i64;
    }
    let seek_whence = whence & !ffmpeg_next::ffi::AVSEEK_FORCE;
    let style = match seek_whence {
        0 => SeekFrom::Start(offset.max(0) as u64), // SEEK_SET
        1 => SeekFrom::Current(offset),             // SEEK_CUR
        2 => SeekFrom::End(offset),                 // SEEK_END
        _ => return -1,
    };
    match stream.seek(style) {
        Ok(new_pos) => new_pos as i64,
        Err(_) => -1,
    }
}

/// Opens a `ResolvedIsoMedia` virtual stream as an `ffmpeg_next::format::context::Input`
/// backed by a custom `AVIOContext`.
pub fn open_ffmpeg_input_from_iso(
    resolved: &ResolvedIsoMedia,
    hint_url: &str,
) -> Result<(ffmpeg_next::format::context::Input, CustomAvioGuard)> {
    let virtual_stream = IsoVirtualStream::new(resolved.extents.clone())?;
    let opaque_ptr = Box::into_raw(Box::new(virtual_stream));

    unsafe {
        let io_buf_size: std::ffi::c_int = 64 * 1024;
        let io_buf = ffmpeg_next::ffi::av_malloc(io_buf_size as usize) as *mut u8;
        if io_buf.is_null() {
            drop(Box::from_raw(opaque_ptr));
            return Err(anyhow::anyhow!("av_malloc failed for ISO AVIOContext"));
        }

        let avio_ctx = ffmpeg_next::ffi::avio_alloc_context(
            io_buf,
            io_buf_size,
            0,
            opaque_ptr as *mut std::ffi::c_void,
            Some(avio_read_packet),
            None,
            Some(avio_seek_packet),
        );
        if avio_ctx.is_null() {
            ffmpeg_next::ffi::av_free(io_buf as *mut std::ffi::c_void);
            drop(Box::from_raw(opaque_ptr));
            return Err(anyhow::anyhow!("avio_alloc_context failed"));
        }

        let guard = CustomAvioGuard {
            avio_ctx,
            opaque_ptr,
        };

        let mut ps = ffmpeg_next::ffi::avformat_alloc_context();
        if ps.is_null() {
            return Err(anyhow::anyhow!("avformat_alloc_context failed"));
        }
        (*ps).pb = avio_ctx;
        (*ps).flags |= ffmpeg_next::ffi::AVFMT_FLAG_CUSTOM_IO;

        let mut dict = ffmpeg_next::Dictionary::new();
        // Use a larger 32 MB probesize / 15s analyzeduration for DVD VOB/AOB MPEG-PS streams
        // so all LPCM, AC-3, and DTS audio tracks are discovered past studio logos.
        dict.set("probesize", "32000000");
        dict.set("analyzeduration", "15000000");
        let mut raw_dict = dict.disown();

        let c_url = std::ffi::CString::new(hint_url).unwrap_or_default();
        let ret = ffmpeg_next::ffi::avformat_open_input(
            &mut ps,
            c_url.as_ptr(),
            std::ptr::null(),
            &mut raw_dict,
        );
        ffmpeg_next::Dictionary::own(raw_dict);

        if ret < 0 || ps.is_null() {
            return Err(anyhow::anyhow!(
                "avformat_open_input failed on ISO stream (err={})",
                ret
            ));
        }

        let info_ret = ffmpeg_next::ffi::avformat_find_stream_info(ps, std::ptr::null_mut());
        if info_ret < 0 {
            ffmpeg_next::ffi::avformat_close_input(&mut ps);
            return Err(anyhow::anyhow!(
                "avformat_find_stream_info failed on ISO stream (err={})",
                info_ret
            ));
        }

        Ok((ffmpeg_next::format::context::Input::wrap(ps), guard))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build a minimal valid ISO 9660 image in memory from a list of `(path_in_video_ts, bytes)`.
    fn build_synthetic_dvd_iso(volume_label: &str, video_ts_files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        // Layout:
        // Sectors 0..16: System Area (zeros)
        // Sector 16: Primary Volume Descriptor (PVD)
        // Sector 17: Volume Descriptor Set Terminator
        // Sector 18: Root Directory (`/` containing `VIDEO_TS`)
        // Sector 19: `VIDEO_TS` Directory
        // Sector 20..: File extents
        let mut current_lba = 20u32;
        let mut file_placements: Vec<(&str, u32, u32)> = Vec::new();
        for (name, data) in video_ts_files {
            let len = data.len() as u32;
            let sectors = (len as u64).div_ceil(DVD_SECTOR_SIZE).max(1) as u32;
            file_placements.push((*name, current_lba, len));
            current_lba += sectors;
        }

        let total_sectors = current_lba as usize;
        let mut iso = vec![0u8; total_sectors * (DVD_SECTOR_SIZE as usize)];

        fn write_both_u32(dst: &mut [u8], val: u32) {
            dst[0..4].copy_from_slice(&val.to_le_bytes());
            dst[4..8].copy_from_slice(&val.to_be_bytes());
        }

        fn push_dir_record(
            buf: &mut [u8],
            cursor: &mut usize,
            lba: u32,
            data_len: u32,
            is_dir: bool,
            name: &[u8],
        ) {
            let rec_len = (33 + name.len() + name.len().is_multiple_of(2) as usize) as u8;
            let rec = &mut buf[*cursor..*cursor + rec_len as usize];
            rec[0] = rec_len;
            rec[1] = 0;
            write_both_u32(&mut rec[2..10], lba);
            write_both_u32(&mut rec[10..18], data_len);
            rec[25] = if is_dir { 0x02 } else { 0x00 };
            rec[28] = 1;
            rec[31] = 1;
            rec[32] = name.len() as u8;
            rec[33..33 + name.len()].copy_from_slice(name);
            *cursor += rec_len as usize;
        }

        // Sector 18: Root directory
        {
            let root_dir = &mut iso[18 * 2048..19 * 2048];
            let mut cur = 0;
            push_dir_record(root_dir, &mut cur, 18, 2048, true, &[0]);
            push_dir_record(root_dir, &mut cur, 18, 2048, true, &[1]);
            push_dir_record(root_dir, &mut cur, 19, 2048, true, b"VIDEO_TS");
        }

        // Sector 19: VIDEO_TS directory
        {
            let vts_dir = &mut iso[19 * 2048..20 * 2048];
            let mut cur = 0;
            push_dir_record(vts_dir, &mut cur, 19, 2048, true, &[0]);
            push_dir_record(vts_dir, &mut cur, 18, 2048, true, &[1]);
            for &(name, lba, len) in &file_placements {
                let iso_name = format!("{};1", name);
                push_dir_record(vts_dir, &mut cur, lba, len, false, iso_name.as_bytes());
            }
        }

        // Sector 16: PVD
        {
            let pvd = &mut iso[16 * 2048..17 * 2048];
            pvd[0] = 1;
            pvd[1..6].copy_from_slice(b"CD001");
            pvd[6] = 1;
            let mut label_bytes = [b' '; 32];
            let src = volume_label.as_bytes();
            let n = src.len().min(32);
            label_bytes[..n].copy_from_slice(&src[..n]);
            pvd[40..72].copy_from_slice(&label_bytes);
            write_both_u32(&mut pvd[80..88], total_sectors as u32);
            let mut cur = 156;
            push_dir_record(pvd, &mut cur, 18, 2048, true, &[0]);
        }

        // Sector 17: Terminator
        {
            let term = &mut iso[17 * 2048..18 * 2048];
            term[0] = 255;
            term[1..6].copy_from_slice(b"CD001");
            term[6] = 1;
        }

        // File data
        for ((_, data), &(_, lba, _)) in video_ts_files.iter().zip(file_placements.iter()) {
            let start = (lba as usize) * 2048;
            iso[start..start + data.len()].copy_from_slice(data);
        }

        iso
    }

    /// Builds a synthetic `VTS_01_0.IFO` (2 sectors = 4096 bytes) with:
    /// - 2 Audio Streams: Track 1 LPCM 2.0 48kHz 16-bit `[en]`, Track 2 AC3 5.1 48kHz `[en]`
    /// - 1 PGC (01:33:15.00 @ 25 fps = 5595.0s) with 2 cells:
    ///   - Cell 0: sectors 0..=1, duration 2795.0s
    ///   - Cell 1: sectors 2..=3, duration 2800.0s
    fn build_synthetic_vts_ifo() -> Vec<u8> {
        let mut ifo = vec![0u8; 4096];
        ifo[0..12].copy_from_slice(b"DVDVIDEO-VTS");
        ifo[0x0C..0x10].copy_from_slice(&7u32.to_be_bytes());
        ifo[0x1C..0x20].copy_from_slice(&1u32.to_be_bytes());
        ifo[0xC4..0xC8].copy_from_slice(&2u32.to_be_bytes());
        ifo[0xCC..0xD0].copy_from_slice(&1u32.to_be_bytes()); // VTS_PGCI at sector 1 (offset 2048)

        // VTS_AST_ATRT at 0x0202: 2 audio streams
        ifo[0x0202..0x0204].copy_from_slice(&2u16.to_be_bytes());
        // Audio 0: LPCM (coding_mode = 4 -> 0x80), lang_type = 1 (0x04), 16-bit 48kHz 2ch (0x01), "en"
        ifo[0x0204] = (4 << 5) | (1 << 2);
        ifo[0x0205] = 0x01; // quant=0 (16-bit), rate=0 (48kHz), ch=1 (2ch)
        ifo[0x0206..0x0208].copy_from_slice(b"en");
        // Audio 1: AC3 (coding_mode = 0), lang_type = 1 (0x04), 48kHz 6ch (0x05), "en"
        ifo[0x020C] = 1 << 2;
        ifo[0x020D] = 0x05; // 6ch (5.1)
        ifo[0x020E..0x0210].copy_from_slice(b"en");

        // Sector 1 (offset 2048): VTS_PGCI
        let pgci = &mut ifo[2048..4096];
        pgci[0..2].copy_from_slice(&1u16.to_be_bytes()); // 1 PGC
        pgci[4..8].copy_from_slice(&512u32.to_be_bytes());
        pgci[8] = 0x81; // Entry PGC for Title 1
        pgci[12..16].copy_from_slice(&16u32.to_be_bytes()); // PGC starts at pgci + 16

        let pgc_base = 16usize;
        pgci[pgc_base + 2] = 2; // 2 programs
        pgci[pgc_base + 3] = 2; // 2 cells
        // Playback time: 01:33:15.00 @ 25 fps (BCD: 0x01, 0x33, 0x15, 0x40) = 3600 + 1980 + 15 = 5595.0s
        pgci[pgc_base + 4..pgc_base + 8].copy_from_slice(&[0x01, 0x33, 0x15, 0x40]);
        // c_pbit_offset at pgc_base + 0xE8 -> 0x0100
        pgci[pgc_base + 0xE8..pgc_base + 0xEA].copy_from_slice(&0x0100u16.to_be_bytes());

        // Cell 0 at pgc_base + 0x100: 00:46:35.00 (2795.0s), first_sector=0, last_sector=1
        let c0 = pgc_base + 0x100;
        pgci[c0 + 4..c0 + 8].copy_from_slice(&[0x00, 0x46, 0x35, 0x40]);
        pgci[c0 + 8..c0 + 12].copy_from_slice(&0u32.to_be_bytes());
        pgci[c0 + 20..c0 + 24].copy_from_slice(&1u32.to_be_bytes());

        // Cell 1 at pgc_base + 0x118: 00:46:40.00 (2800.0s), first_sector=2, last_sector=3
        let c1 = c0 + 24;
        pgci[c1 + 4..c1 + 8].copy_from_slice(&[0x00, 0x46, 0x40, 0x40]);
        pgci[c1 + 8..c1 + 12].copy_from_slice(&2u32.to_be_bytes());
        pgci[c1 + 20..c1 + 24].copy_from_slice(&3u32.to_be_bytes());

        ifo
    }

    #[test]
    fn test_parse_iso_and_vts_ifo_and_virtual_stream() {
        let ifo_bytes = build_synthetic_vts_ifo();
        let mut vob1 = vec![0xAAu8; 4096]; // 2 sectors (sectors 0..=1)
        vob1[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0xBA]);
        let mut vob2 = vec![0x55u8; 4096]; // 2 sectors (sectors 2..=3)
        vob2[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0xBB]);

        let iso_bytes = build_synthetic_dvd_iso(
            "PORTISHEAD_PNVC",
            &[
                ("VTS_01_0.IFO", ifo_bytes),
                ("VTS_01_1.VOB", vob1.clone()),
                ("VTS_01_2.VOB", vob2.clone()),
            ],
        );

        let temp_iso = std::env::temp_dir().join("rusttracker_test_portishead_pnvc.iso");
        std::fs::write(&temp_iso, &iso_bytes).expect("write synthetic ISO");

        let resolved = resolve_iso_or_vob_media(&temp_iso)
            .expect("resolve_iso_or_vob_media")
            .expect("Expected Some(ResolvedIsoMedia)");

        assert_eq!(resolved.volume_label.as_deref(), Some("PORTISHEAD_PNVC"));
        assert_eq!(resolved.media_kind, IsoMediaKind::DvdVideo);
        assert_eq!(resolved.total_bytes, 8192);
        assert_eq!(resolved.ifo_duration_sec, Some(5595.0));
        assert_eq!(resolved.cells.len(), 2);
        assert_eq!(resolved.audio_streams.len(), 2);
        assert_eq!(resolved.audio_streams[0].codec, "LPCM");
        assert_eq!(resolved.audio_streams[0].channels, 2);
        assert_eq!(resolved.audio_streams[0].bit_depth, Some(16));
        assert_eq!(resolved.audio_streams[0].language.as_deref(), Some("en"));
        assert_eq!(resolved.audio_streams[1].codec, "AC3");
        assert_eq!(resolved.audio_streams[1].channels, 6);

        // Verify cell seek mapping: seeking to start of Cell 1 (2795.0s) maps to byte 4096 (start of VTS_01_2.VOB)
        assert_eq!(resolved.time_to_logical_byte(0.0, 5595.0), 0);
        assert_eq!(resolved.time_to_logical_byte(2795.0, 5595.0), 4096);

        // Verify IsoVirtualStream reads seamlessly across VTS_01_1.VOB and VTS_01_2.VOB
        let mut vs =
            IsoVirtualStream::new(resolved.extents.clone()).expect("IsoVirtualStream::new");
        let mut all_read = Vec::new();
        vs.read_to_end(&mut all_read).expect("read_to_end");
        assert_eq!(all_read.len(), 8192);
        assert_eq!(&all_read[0..4096], &vob1[..]);
        assert_eq!(&all_read[4096..8192], &vob2[..]);

        let _ = std::fs::remove_file(&temp_iso);
    }

    #[test]
    fn test_load_audio_source_from_multi_extent_iso() {
        // Build a valid 48kHz stereo 16-bit PCM WAV stream of 8192 bytes (2 sectors per VOB extent)
        let pcm_bytes_len: u32 = 8192 - 44;
        let total_riff_len: u32 = 36 + pcm_bytes_len;
        let mut wav = Vec::with_capacity(8192);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&total_riff_len.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&2u16.to_le_bytes()); // Stereo
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&(48000u32 * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&pcm_bytes_len.to_le_bytes());
        for i in 0..(pcm_bytes_len as usize / 2) {
            let sample = ((i as f32 * 0.1).sin() * 12000.0) as i16;
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        assert_eq!(wav.len(), 8192);

        let ifo_bytes = build_synthetic_vts_ifo();
        let vob1 = wav[0..4096].to_vec();
        let vob2 = wav[4096..8192].to_vec();

        let iso_bytes = build_synthetic_dvd_iso(
            "PORTISHEAD_PNVC",
            &[
                ("VTS_01_0.IFO", ifo_bytes),
                ("VTS_01_1.VOB", vob1),
                ("VTS_01_2.VOB", vob2),
            ],
        );

        let temp_iso = std::env::temp_dir().join("rusttracker_test_playback_portishead.iso");
        std::fs::write(&temp_iso, &iso_bytes).expect("write playback ISO");

        let mut source = crate::audio::load_audio_source(temp_iso.to_str().unwrap())
            .expect("load_audio_source on synthetic DVD ISO");
        assert_eq!(source.get_artist(), "PORTISHEAD_PNVC");
        assert_eq!(source.get_duration_seconds(), 5595.0);
        assert_eq!(source.get_num_channels(), 2);

        let mut out = vec![0.0f32; 1024 * 2];
        let frames = source.read_frames(2, 48000, &mut out);
        assert!(frames > 0, "Expected decoded audio frames from ISO stream");

        source.set_position_seconds(2795.0);
        assert_eq!(source.get_position_seconds(), 2795.0);

        drop(source);
        let _ = std::fs::remove_file(&temp_iso);
    }
}
