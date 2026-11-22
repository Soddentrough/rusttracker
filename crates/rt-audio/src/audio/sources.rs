use super::*;
use anyhow::{Context, Result};
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use rt_decode::openmpt_ffi::OpenMptModule;
use rustysynth::{MidiFile, MidiFileSequencer, SoundFont, Synthesizer, SynthesizerSettings};
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{
    AudioDecoder, AudioDecoderOptions, CODEC_ID_NULL_AUDIO, well_known as audio_codecs,
};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

// ---------------------------------------------------------
// OpenMPT Tracker Decoder (Direct Zero-Allocation C FFI)
// ---------------------------------------------------------
pub(in crate::audio) struct OpenMptSource {
    pub(in crate::audio) module: OpenMptModule,
    pub(in crate::audio) volume: f32,
}

impl AudioSource for OpenMptSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        if hardware_channels == 0 {
            return 0;
        }
        let frames_to_render = output.len() / hardware_channels;
        if frames_to_render == 0 {
            return 0;
        }

        let vol = self.volume;
        if hardware_channels == 1 {
            let frames_read = self
                .module
                .read_float_mono(sample_rate as i32, &mut output[..frames_to_render]);
            if (vol - 1.0).abs() > f32::EPSILON {
                for s in &mut output[..frames_read] {
                    *s *= vol;
                }
            }
            return frames_read;
        }

        // Render interleaved (L, R) directly into the front of `output` with zero heap allocations.
        let frames_read = self
            .module
            .read_interleaved_float_stereo(sample_rate as i32, &mut output[..frames_to_render * 2]);

        if hardware_channels == 2 {
            if (vol - 1.0).abs() > f32::EPSILON {
                for s in &mut output[..frames_read * 2] {
                    *s *= vol;
                }
            }
        } else {
            // Expand stereo (2ch) -> multichannel (>2ch) backwards in-place so unread stereo
            // pairs at `i * 2` are never overwritten before being read (`i * hw_ch >= i * 2`).
            for i in (0..frames_read).rev() {
                let l = output[i * 2] * vol;
                let r = output[i * 2 + 1] * vol;
                let dst = &mut output[i * hardware_channels..(i + 1) * hardware_channels];
                dst[0] = l;
                dst[1] = r;
                dst[2..].fill(0.0);
            }
        }

        frames_read
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.module.duration_seconds()
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.module.position_seconds()
    }
    fn set_position_seconds(&mut self, pos: f64) {
        self.module.set_position_seconds(pos);
    }
    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        self.module.current_channel_vu_mono(channel) * self.volume
    }
    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if let Some(&(_, vol)) = tracks.first() {
            self.volume = vol.clamp(0.0, 2.0);
        } else {
            self.volume = 0.0;
        }
        Ok(())
    }

    fn get_artist(&mut self) -> String {
        self.module
            .metadata(c"artist")
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn get_type(&mut self) -> String {
        self.module
            .metadata(c"type")
            .unwrap_or_else(|| "Tracker".to_string())
    }

    fn get_tempo(&mut self) -> i32 {
        self.module.current_tempo()
    }
    fn get_speed(&mut self) -> i32 {
        self.module.current_speed()
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        None
    }
    fn get_num_samples(&mut self) -> i32 {
        self.module.num_samples()
    }
    fn get_num_instruments(&mut self) -> i32 {
        self.module.num_instruments()
    }
    fn get_num_patterns(&mut self) -> i32 {
        self.module.num_patterns()
    }
    fn get_current_order(&mut self) -> i32 {
        self.module.current_order()
    }
    fn get_current_row(&mut self) -> i32 {
        self.module.current_row()
    }
    fn get_tracker_channels(&mut self) -> Option<i32> {
        Some(self.module.num_channels())
    }

    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        self.module.pre_format_all_patterns()
    }

    fn get_current_row_string(&mut self) -> String {
        self.module.current_row_string()
    }
}

// ---------------------------------------------------------
// MidiTracker Audio Decoder
// ---------------------------------------------------------
#[derive(Clone)]
struct MidiEvent {
    time_sec: f64,
    channel: i32,
    velocity: u8,
    is_note_on: bool,
}

struct MidiSource {
    sequencer: Option<MidiFileSequencer>,
    sf2_arc: Option<Arc<SoundFont>>,
    synth_sample_rate: u32,
    volume: f32,
    pos_silent: f64,
    events: Vec<MidiEvent>,
    event_idx: usize,
    channel_vus: Vec<f32>,
    duration: f64,
    artist: String,
    title: String,
    tracker_data: Vec<Vec<String>>,
    tempo: i32,
    tempo_map: Vec<(u32, f64, f64)>,
    ticks_per_beat: f64,
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
    midi_file: Arc<MidiFile>,
}

unsafe impl Send for MidiSource {}

impl MidiSource {
    pub fn new(file_path: &str, soundfont_path: &str, sample_rate: i32) -> anyhow::Result<Self> {
        type CachedSoundFont = std::sync::Mutex<Option<(String, Arc<SoundFont>)>>;
        static CACHED_SOUNDFONT: std::sync::OnceLock<CachedSoundFont> = std::sync::OnceLock::new();
        let cache = CACHED_SOUNDFONT.get_or_init(|| std::sync::Mutex::new(None));
        let sf2_arc_opt = {
            let mut guard = cache.lock().unwrap();
            if let Some((cached_path, sf2)) = guard.as_ref()
                && cached_path == soundfont_path
            {
                Some(sf2.clone())
            } else if let Ok(mut sf2_file) = File::open(soundfont_path)
                && let Ok(sf2) = SoundFont::new(&mut sf2_file)
            {
                let arc = Arc::new(sf2);
                *guard = Some((soundfont_path.to_string(), arc.clone()));
                Some(arc)
            } else {
                None
            }
        };

        let mut artist_suffix = "";
        let mut sequencer = if let Some(ref sf2) = sf2_arc_opt {
            let settings = SynthesizerSettings::new(sample_rate);
            if let Ok(synth) = Synthesizer::new(sf2, &settings) {
                Some(MidiFileSequencer::new(synth))
            } else {
                artist_suffix = " (No SoundFont)";
                None
            }
        } else {
            artist_suffix = " (No SoundFont)";
            None
        };

        let mut midi_file = File::open(file_path)?;
        let midi = Arc::new(
            MidiFile::new(&mut midi_file)
                .map_err(|e| anyhow::anyhow!("Midi parse error: {:?}", e))?,
        );

        let duration = midi.get_length();
        if let Some(seq) = &mut sequencer {
            seq.play(&midi, false);
        }

        let data = std::fs::read(file_path)?;
        let smf = Smf::parse(&data).map_err(|e| anyhow::anyhow!("Midly parse error: {:?}", e))?;

        let mut artist = String::new();
        let mut title = String::new();

        let mut absolute_events = Vec::new();
        let mut raw_tempo_changes: Vec<(u32, f64)> = Vec::new();
        let ticks_per_beat = match smf.header.timing {
            midly::Timing::Metrical(ticks) => ticks.as_int() as f64,
            _ => 480.0,
        };

        for track in &smf.tracks {
            let mut current_tick = 0;
            for event in track {
                current_tick += event.delta.as_int();
                match &event.kind {
                    TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                        if title.is_empty() {
                            title = String::from_utf8_lossy(name).to_string();
                        }
                    }
                    TrackEventKind::Meta(MetaMessage::Text(text)) => {
                        if artist.is_empty() {
                            artist = String::from_utf8_lossy(text).to_string();
                        }
                    }
                    TrackEventKind::Meta(MetaMessage::Tempo(micros)) => {
                        let us = micros.as_int().max(1) as f64;
                        raw_tempo_changes.push((current_tick, us));
                    }
                    TrackEventKind::Midi { channel, message } => match message {
                        MidiMessage::NoteOn { key, vel } => {
                            absolute_events.push((
                                current_tick,
                                channel.as_int(),
                                key.as_int(),
                                vel.as_int(),
                                vel.as_int() > 0,
                            ));
                        }
                        MidiMessage::NoteOff { key, vel } => {
                            absolute_events.push((
                                current_tick,
                                channel.as_int(),
                                key.as_int(),
                                vel.as_int(),
                                false,
                            ));
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        // Permissive SMF fallback for files (e.g., XMI2MID conversions) where midly stops early
        // due to 8-bit controller values like `B1 00 FF`.
        if absolute_events.is_empty() && data.len() >= 14 && &data[0..4] == b"MThd" {
            let mut pos = 8 + u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
            while pos + 8 <= data.len() {
                let chunk_id = &data[pos..pos + 4];
                let chunk_len = u32::from_be_bytes([
                    data[pos + 4],
                    data[pos + 5],
                    data[pos + 6],
                    data[pos + 7],
                ]) as usize;
                pos += 8;
                let track_end = (pos + chunk_len).min(data.len());
                if chunk_id == b"MTrk" {
                    let mut p = pos;
                    let mut tick = 0u32;
                    let mut last_status = 0u8;
                    let read_varlen = |buf: &[u8], idx: &mut usize, end: usize| -> Option<u32> {
                        let mut val = 0u32;
                        for _ in 0..4 {
                            if *idx >= end {
                                return None;
                            }
                            let b = buf[*idx];
                            *idx += 1;
                            val = (val << 7) | ((b & 0x7F) as u32);
                            if (b & 0x80) == 0 {
                                return Some(val);
                            }
                        }
                        Some(val)
                    };
                    while p < track_end {
                        let Some(delta) = read_varlen(&data, &mut p, track_end) else {
                            break;
                        };
                        tick = tick.saturating_add(delta);
                        if p >= track_end {
                            break;
                        }
                        let first = data[p];
                        p += 1;
                        if (first & 0x80) == 0 {
                            let cmd = last_status & 0xF0;
                            let ch = last_status & 0x0F;
                            let d1 = first & 0x7F;
                            if cmd == 0xC0 || cmd == 0xD0 {
                                continue;
                            }
                            if p >= track_end {
                                break;
                            }
                            let d2 = data[p] & 0x7F;
                            p += 1;
                            if cmd == 0x90 {
                                absolute_events.push((tick, ch, d1, d2, d2 > 0));
                            } else if cmd == 0x80 {
                                absolute_events.push((tick, ch, d1, d2, false));
                            }
                        } else if first == 0xF0 || first == 0xF7 {
                            let Some(len) = read_varlen(&data, &mut p, track_end) else {
                                break;
                            };
                            p = (p + len as usize).min(track_end);
                        } else if first == 0xFF {
                            if p >= track_end {
                                break;
                            }
                            let meta_type = data[p];
                            p += 1;
                            let Some(len) = read_varlen(&data, &mut p, track_end) else {
                                break;
                            };
                            let meta_end = (p + len as usize).min(track_end);
                            let slice = &data[p..meta_end];
                            p = meta_end;
                            if meta_type == 0x2F {
                                break;
                            } else if meta_type == 0x51 && slice.len() == 3 {
                                let us = u32::from_be_bytes([0, slice[0], slice[1], slice[2]])
                                    .max(1) as f64;
                                raw_tempo_changes.push((tick, us));
                            } else if meta_type == 0x03 && title.is_empty() {
                                title = String::from_utf8_lossy(slice).to_string();
                            } else if meta_type == 0x01 && artist.is_empty() {
                                artist = String::from_utf8_lossy(slice).to_string();
                            }
                        } else {
                            last_status = first;
                            let cmd = first & 0xF0;
                            let ch = first & 0x0F;
                            if p >= track_end {
                                break;
                            }
                            let d1 = data[p] & 0x7F;
                            p += 1;
                            if cmd == 0xC0 || cmd == 0xD0 {
                                continue;
                            }
                            if p >= track_end {
                                break;
                            }
                            let d2 = data[p] & 0x7F;
                            p += 1;
                            if cmd == 0x90 {
                                absolute_events.push((tick, ch, d1, d2, d2 > 0));
                            } else if cmd == 0x80 {
                                absolute_events.push((tick, ch, d1, d2, false));
                            }
                        }
                    }
                }
                pos = track_end;
            }
        }

        raw_tempo_changes.sort_by_key(|e| e.0);
        let mut tempo_map: Vec<(u32, f64, f64)> = vec![(0, 0.0, 500_000.0)];
        for (tick, us_per_beat) in raw_tempo_changes {
            let &(prev_tick, prev_sec, prev_us) = tempo_map.last().unwrap();
            if tick == prev_tick {
                tempo_map.last_mut().unwrap().2 = us_per_beat;
            } else {
                let delta_beats = (tick - prev_tick) as f64 / ticks_per_beat;
                let sec = prev_sec + delta_beats * (prev_us / 1_000_000.0);
                tempo_map.push((tick, sec, us_per_beat));
            }
        }

        let initial_bpm = (60_000_000.0 / tempo_map[0].2).round() as i32;

        absolute_events.sort_by_key(|e| e.0);

        let mut parsed_events = Vec::with_capacity(absolute_events.len());
        let mut pattern_rows: Vec<Vec<String>> = Vec::new();
        let notes = [
            "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
        ];
        let mut tempo_idx = 0;
        for (tick, channel, key, vel, is_on) in absolute_events {
            while tempo_idx + 1 < tempo_map.len() && tempo_map[tempo_idx + 1].0 <= tick {
                tempo_idx += 1;
            }
            let (base_tick, base_sec, us_per_beat) = tempo_map[tempo_idx];
            let delta_ticks = tick.saturating_sub(base_tick);
            let beats = delta_ticks as f64 / ticks_per_beat;
            let current_time = base_sec + beats * (us_per_beat / 1_000_000.0);

            parsed_events.push(MidiEvent {
                time_sec: current_time,
                channel: channel as i32,
                velocity: vel,
                is_note_on: is_on,
            });

            if is_on && (channel as usize) < 16 {
                let raw_row = tick as f64 / (ticks_per_beat / 4.0);
                const MAX_MIDI_PATTERN_ROWS: usize = 65_536;
                if raw_row.is_finite() && raw_row >= 0.0 && raw_row < MAX_MIDI_PATTERN_ROWS as f64 {
                    let row_idx = raw_row as usize;
                    while pattern_rows.len() <= row_idx {
                        pattern_rows.push(vec!["... .. ..".to_string(); 16]);
                    }
                    let octave = key / 12;
                    let note = key % 12;
                    let note_name = format!("{}{}", notes[note as usize], octave);
                    let note_str = format!("{} .. {:02X}", note_name, vel);
                    pattern_rows[row_idx][channel as usize] = note_str;
                }
            }
        }

        let mut final_rows = Vec::new();
        for row in pattern_rows {
            final_rows.push(row.join(" | "));
        }
        let tracker_data = vec![final_rows];

        if artist.is_empty() {
            artist = "Unknown MIDI".to_string();
        }
        if !artist_suffix.is_empty() {
            artist.push_str(artist_suffix);
        }

        Ok(Self {
            sequencer,
            sf2_arc: sf2_arc_opt,
            synth_sample_rate: sample_rate.max(8000) as u32,
            volume: 1.0,
            pos_silent: 0.0,
            events: parsed_events,
            event_idx: 0,
            channel_vus: vec![0.0; 16],
            duration,
            artist,
            title,
            tracker_data,
            tempo: initial_bpm,
            tempo_map,
            ticks_per_beat,
            left_buf: vec![0.0; 8192],
            right_buf: vec![0.0; 8192],
            midi_file: midi,
        })
    }
}

impl AudioSource for MidiSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let frames_to_render = output.len() / hardware_channels;
        let target_sr = sample_rate.max(8000);
        if target_sr != self.synth_sample_rate {
            let cur_pos = self.get_position_seconds();
            if let Some(ref sf2) = self.sf2_arc {
                let settings = SynthesizerSettings::new(target_sr as i32);
                if let Ok(synth) = Synthesizer::new(sf2, &settings) {
                    let mut seq = MidiFileSequencer::new(synth);
                    seq.play(&self.midi_file, false);
                    let mut trash_left = vec![0.0; 8192];
                    let mut trash_right = vec![0.0; 8192];
                    while seq.get_position() < cur_pos && !seq.end_of_sequence() {
                        seq.render(&mut trash_left, &mut trash_right);
                    }
                    self.sequencer = Some(seq);
                }
            }
            self.synth_sample_rate = target_sr;
        }

        let vol = self.volume;
        let pos = if let Some(ref mut seq) = self.sequencer {
            if self.left_buf.len() < frames_to_render {
                self.left_buf.resize(frames_to_render, 0.0);
                self.right_buf.resize(frames_to_render, 0.0);
            }
            seq.render(
                &mut self.left_buf[..frames_to_render],
                &mut self.right_buf[..frames_to_render],
            );
            for i in 0..frames_to_render {
                let l = self.left_buf[i] * vol;
                let r = self.right_buf[i] * vol;
                if hardware_channels == 1 {
                    output[i] = (l + r) * 0.5;
                } else {
                    output[i * hardware_channels] = l;
                    output[i * hardware_channels + 1] = r;
                    for j in 2..hardware_channels {
                        output[i * hardware_channels + j] = 0.0;
                    }
                }
            }
            seq.get_position()
        } else {
            output.fill(0.0);
            self.pos_silent =
                (self.pos_silent + frames_to_render as f64 / target_sr as f64).min(self.duration);
            self.pos_silent
        };

        let dt = frames_to_render as f32 / target_sr as f32;
        let decay = 10.0_f32.powf(-13.3 * dt / 20.0);
        for ch in &mut self.channel_vus {
            *ch *= decay;
            if *ch < 1e-4 {
                *ch = 0.0;
            }
        }

        while self.event_idx < self.events.len() && self.events[self.event_idx].time_sec <= pos {
            let ev = &self.events[self.event_idx];
            if ev.is_note_on {
                let ch = ev.channel as usize;
                if ch < 16 {
                    self.channel_vus[ch] =
                        ((ev.velocity as f32 / 127.0).clamp(0.1, 1.0) * vol).clamp(0.0, 1.0);
                }
            }
            self.event_idx += 1;
        }

        let is_end = if let Some(ref seq) = self.sequencer {
            seq.end_of_sequence()
        } else {
            pos >= self.duration
        };

        if is_end { 0 } else { frames_to_render }
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        if let Some(ref seq) = self.sequencer {
            seq.get_position()
        } else {
            self.pos_silent
        }
    }

    fn set_position_seconds(&mut self, pos: f64) {
        if let Some(ref mut seq) = self.sequencer {
            if pos < seq.get_position() {
                seq.play(&self.midi_file, false);
            }

            let mut trash_left = vec![0.0; 8192];
            let mut trash_right = vec![0.0; 8192];
            while seq.get_position() < pos && !seq.end_of_sequence() {
                seq.render(&mut trash_left, &mut trash_right);
            }
        } else {
            self.pos_silent = pos.clamp(0.0, self.duration);
        }

        self.event_idx = 0;
        while self.event_idx < self.events.len() && self.events[self.event_idx].time_sec < pos {
            self.event_idx += 1;
        }
        self.channel_vus.fill(0.0);
    }

    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        if (0..16).contains(&channel) {
            self.channel_vus[channel as usize]
        } else {
            0.0
        }
    }
    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if let Some(&(_, vol)) = tracks.first() {
            self.volume = vol.clamp(0.0, 2.0);
        } else {
            self.volume = 0.0;
        }
        Ok(())
    }

    fn get_artist(&mut self) -> String {
        if !self.title.is_empty() {
            format!("{} - {}", self.artist, self.title)
        } else {
            self.artist.clone()
        }
    }

    fn get_type(&mut self) -> String {
        "MIDI".to_string()
    }
    fn get_tempo(&mut self) -> i32 {
        let pos = self.get_position_seconds();
        let mut us = self.tempo_map.first().map(|e| e.2).unwrap_or(500_000.0);
        for &(_, sec, tempo_us) in &self.tempo_map {
            if sec <= pos {
                us = tempo_us;
            } else {
                break;
            }
        }
        self.tempo = (60_000_000.0 / us).round() as i32;
        self.tempo
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        None
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        128
    }
    fn get_num_patterns(&mut self) -> i32 {
        1
    }

    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        let pos = self.get_position_seconds();
        let mut base_tick = 0u32;
        let mut base_sec = 0.0;
        let mut us = 500_000.0;
        for &(t, sec, tempo_us) in &self.tempo_map {
            if sec <= pos {
                base_tick = t;
                base_sec = sec;
                us = tempo_us;
            } else {
                break;
            }
        }
        let delta_sec = (pos - base_sec).max(0.0);
        let delta_ticks = (delta_sec / (us / 1_000_000.0)) * self.ticks_per_beat;
        let current_tick = base_tick as f64 + delta_ticks;
        (current_tick / (self.ticks_per_beat / 4.0)) as i32
    }

    fn get_tracker_channels(&mut self) -> Option<i32> {
        Some(16)
    }

    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        self.tracker_data.clone()
    }

    fn get_current_row_string(&mut self) -> String {
        let row = self.get_current_row() as usize;
        if !self.tracker_data.is_empty() && row < self.tracker_data[0].len() {
            self.tracker_data[0][row].clone()
        } else {
            vec!["... .. ..".to_string(); 16].join(" | ")
        }
    }
}

// ---------------------------------------------------------
// Symphonia Standard Audio Decoder
// ---------------------------------------------------------
pub(crate) struct StreamingSincResampler {
    in_rate: u32,
    out_rate: u32,
    channels: usize,
    resampler: rubato::Async<f32>,
    in_staging: Vec<f32>,
    in_pos: usize,
    out_scratch: Vec<f32>,
}

impl StreamingSincResampler {
    pub(crate) fn new(in_rate: u32, out_rate: u32, channels: usize) -> Result<Self> {
        use rubato::{Async, FixedAsync, SincInterpolationParameters, WindowFunction};
        let in_rate = in_rate.max(1);
        let out_rate = out_rate.max(1);
        let channels = channels.max(1);
        let chunk_frames = 256usize;
        let ratio = out_rate as f64 / in_rate as f64;
        let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2);
        let resampler = Async::<f32>::new_sinc(
            ratio,
            1.1,
            &params,
            chunk_frames,
            channels,
            FixedAsync::Input,
        )
        .map_err(|e| anyhow::anyhow!("Failed to initialize sinc resampler: {e}"))?;
        let out_cap = rubato::Resampler::output_frames_max(&resampler) * channels;
        Ok(Self {
            in_rate,
            out_rate,
            channels,
            resampler,
            in_staging: Vec::with_capacity(chunk_frames * channels * 4),
            in_pos: 0,
            out_scratch: vec![0.0; out_cap],
        })
    }

    pub(crate) fn push_interleaved(&mut self, src: &[f32], dst: &mut Vec<f32>) {
        use audioadapter_buffers::direct::InterleavedSlice;
        use rubato::Resampler;

        if src.is_empty() {
            return;
        }
        if self.in_pos > 4096 {
            self.in_staging.drain(0..self.in_pos);
            self.in_pos = 0;
        }
        self.in_staging.extend_from_slice(src);

        loop {
            let needed = self.resampler.input_frames_next();
            let avail_frames = (self.in_staging.len() - self.in_pos) / self.channels;
            if avail_frames < needed {
                break;
            }
            let out_max = self.resampler.output_frames_max();
            if self.out_scratch.len() < out_max * self.channels {
                self.out_scratch.resize(out_max * self.channels, 0.0);
            }
            let Ok(in_adapter) =
                InterleavedSlice::new(&self.in_staging[self.in_pos..], self.channels, needed)
            else {
                break;
            };
            let Ok(mut out_adapter) =
                InterleavedSlice::new_mut(&mut self.out_scratch, self.channels, out_max)
            else {
                break;
            };
            match self
                .resampler
                .process_into_buffer(&in_adapter, &mut out_adapter, None)
            {
                Ok((consumed, produced)) => {
                    self.in_pos += consumed * self.channels;
                    if produced > 0 {
                        dst.extend_from_slice(&self.out_scratch[..produced * self.channels]);
                    }
                    if consumed == 0 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    pub(crate) fn flush(&mut self, dst: &mut Vec<f32>) {
        use audioadapter_buffers::direct::InterleavedSlice;
        use rubato::{Indexing, Resampler};

        let avail_frames = (self.in_staging.len() - self.in_pos) / self.channels;
        if avail_frames == 0 {
            return;
        }
        let needed = self.resampler.input_frames_next();
        let rem_frames = avail_frames.min(needed.max(1));
        let out_max = self.resampler.output_frames_max();
        if self.out_scratch.len() < out_max * self.channels {
            self.out_scratch.resize(out_max * self.channels, 0.0);
        }
        if let (Ok(in_adapter), Ok(mut out_adapter)) = (
            InterleavedSlice::new(&self.in_staging[self.in_pos..], self.channels, rem_frames),
            InterleavedSlice::new_mut(&mut self.out_scratch, self.channels, out_max),
        ) {
            let idx = Indexing::new().partial_len(rem_frames);
            if let Ok((_consumed, produced)) =
                self.resampler
                    .process_into_buffer(&in_adapter, &mut out_adapter, Some(&idx))
                && produced > 0
            {
                dst.extend_from_slice(&self.out_scratch[..produced * self.channels]);
            }
        }
        self.in_staging.clear();
        self.in_pos = 0;
        self.resampler.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.in_staging.clear();
        self.in_pos = 0;
        rubato::Resampler::reset(&mut self.resampler);
    }

    #[inline]
    pub(crate) fn matches(&self, in_rate: u32, out_rate: u32, channels: usize) -> bool {
        self.in_rate == in_rate && self.out_rate == out_rate && self.channels == channels
    }
}

struct ActiveSymphoniaTrack {
    track_idx: usize,
    track_id: u32,
    decoder: Box<dyn AudioDecoder>,
    sample_buf: Vec<f32>,
    samples: Vec<f32>,
    buf_pos: usize,
    volume: f32,
    enabled: bool,
    channels: u16,
    sample_rate: u32,
    sinc_resampler: Option<StreamingSincResampler>,
    time_base: f64,
    current_time: f64,
}

#[inline(always)]
pub(crate) fn calculate_mix_headroom<I>(volumes: I) -> f32
where
    I: IntoIterator<Item = f32>,
{
    let active_power: f32 = volumes.into_iter().map(|v| v * v).sum();
    if active_power > 1.0 {
        1.0 / active_power.sqrt()
    } else {
        1.0
    }
}

/// Resamples an interleaved multi-channel packet `src` from `in_rate` to `out_rate`
/// using 4-point cubic Hermite (Catmull-Rom) interpolation with seamless $C^1$ continuity
/// across packet boundaries via a 3-frame history buffer (`prev_frame`) and `resample_pos`.
#[cfg(test)]
pub(crate) fn resample_interleaved_packet(
    src: &[f32],
    ch: usize,
    in_rate: u32,
    out_rate: u32,
    resample_pos: &mut f64,
    prev_frame: &mut Vec<f32>,
    dst: &mut Vec<f32>,
) {
    let ch = ch.max(1);
    let in_frames = src.len() / ch;
    if in_frames == 0 {
        return;
    }
    if in_rate == out_rate {
        dst.extend_from_slice(&src[0..in_frames * ch]);
        return;
    }

    let hist_len = 3 * ch;
    if prev_frame.len() != hist_len {
        prev_frame.clear();
        prev_frame.reserve(hist_len);
        for _ in 0..3 {
            prev_frame.extend_from_slice(&src[0..ch]);
        }
        *resample_pos = 0.0;
    }

    let step = in_rate.max(1) as f64 / out_rate.max(1) as f64;
    let limit = in_frames as f64 - 2.0;
    let sample_at = |frame_idx: isize, c: usize, hist: &[f32]| -> f32 {
        if frame_idx < 0 {
            let h_idx = (frame_idx + 3).clamp(0, 2) as usize;
            hist[h_idx * ch + c]
        } else {
            let s_idx = (frame_idx as usize).min(in_frames - 1);
            src[s_idx * ch + c]
        }
    };

    while *resample_pos < limit {
        let idx = (*resample_pos).floor() as isize;
        let frac = (*resample_pos - idx as f64) as f32;
        for c in 0..ch {
            let p0 = sample_at(idx - 1, c, prev_frame);
            let p1 = sample_at(idx, c, prev_frame);
            let p2 = sample_at(idx + 1, c, prev_frame);
            let p3 = sample_at(idx + 2, c, prev_frame);
            let c0 = p1;
            let c1 = 0.5 * (p2 - p0);
            let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
            let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
            dst.push(((c3 * frac + c2) * frac + c1) * frac + c0);
        }
        *resample_pos += step;
    }

    *resample_pos -= in_frames as f64;
    if in_frames >= 3 {
        prev_frame.copy_from_slice(&src[(in_frames - 3) * ch..in_frames * ch]);
    } else if in_frames == 2 {
        prev_frame.copy_within(2 * ch..3 * ch, 0);
        prev_frame[ch..3 * ch].copy_from_slice(&src[0..2 * ch]);
    } else {
        prev_frame.copy_within(ch..3 * ch, 0);
        prev_frame[2 * ch..3 * ch].copy_from_slice(&src[0..ch]);
    }
}

#[inline(always)]
pub fn get_downmixed_sample(
    track_channels: u16,
    base_slice: &[f32],
    hardware_channels: usize,
    out_channel: usize,
) -> f32 {
    if base_slice.is_empty() {
        return 0.0;
    }
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const SQRT2: f32 = std::f32::consts::SQRT_2;

    match (track_channels, hardware_channels, out_channel) {
        // Mono to any channel (duplicate to left and right on stereo+, or mono on 1ch)
        (1, _, c) if c < 2 => base_slice[0],
        (1, _, _) => 0.0,
        // Direct 1-to-1 mapping for matching channel counts or stereo fronts on wider hardware
        (2, hc, c) if hc >= 2 && c < 2 => base_slice.get(c).copied().unwrap_or(0.0),
        (tc, hc, c) if tc as usize == hc && c < tc as usize => {
            base_slice.get(c).copied().unwrap_or(0.0)
        }
        // 3.0 (FL, FR, FC) to 4.0 Quad (FL, FR, BL, BR): fold FC into fronts, keep rears silent
        (3, 4, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (3, 4, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        (3, 4, _) => 0.0,
        // 3.0 (FL, FR, FC) to >= 5ch: [FL, FR, FC, 0...]
        (3, hc, c) if hc >= 5 && c < 3 => base_slice.get(c).copied().unwrap_or(0.0),
        (3, hc, _) if hc >= 5 => 0.0,
        // 4.0 Quad (FL, FR, BL, BR) to 5.0 (FL, FR, FC, Ls, Rs): keep FC silent, route BL/BR -> 3,4
        (4, 5, c @ 0..=1) => base_slice.get(c).copied().unwrap_or(0.0),
        (4, 5, 3) => base_slice.get(2).copied().unwrap_or(0.0),
        (4, 5, 4) => base_slice.get(3).copied().unwrap_or(0.0),
        (4, 5, _) => 0.0,
        // 4.0 Quad (FL, FR, BL, BR) to 5.1 / 7.1 / >=6ch: keep FC/LFE silent, route BL/BR -> 4,5
        (4, hc, c @ 0..=1) if hc >= 6 => base_slice.get(c).copied().unwrap_or(0.0),
        (4, hc, 4) if hc >= 6 => base_slice.get(2).copied().unwrap_or(0.0),
        (4, hc, 5) if hc >= 6 => base_slice.get(3).copied().unwrap_or(0.0),
        (4, hc, _) if hc >= 6 => 0.0,
        // 5.0 (FL, FR, FC, BL, BR) to 6.1 (FL, FR, FC, LFE, BC, SL, SR): keep LFE/BC silent
        (5, 7, 0..=2) => base_slice.get(out_channel).copied().unwrap_or(0.0),
        (5, 7, 5) => base_slice.get(3).copied().unwrap_or(0.0),
        (5, 7, 6) => base_slice.get(4).copied().unwrap_or(0.0),
        (5, 7, _) => 0.0,
        // 5.0 (FL, FR, FC, BL, BR) to >= 6ch (other than 7): keep LFE silent, route BL/BR -> 4,5
        (5, hc, 0..=2) if hc >= 6 => base_slice.get(out_channel).copied().unwrap_or(0.0),
        (5, hc, 4) if hc >= 6 => base_slice.get(3).copied().unwrap_or(0.0),
        (5, hc, 5) if hc >= 6 => base_slice.get(4).copied().unwrap_or(0.0),
        (5, hc, _) if hc >= 6 => 0.0,
        // 5.1 (FL, FR, FC, LFE, Ls, Rs) to 6.1 (FL, FR, FC, LFE, BC, SL, SR)
        (6, 7, 0..=3) => base_slice.get(out_channel).copied().unwrap_or(0.0),
        (6, 7, 5) => base_slice.get(4).copied().unwrap_or(0.0),
        (6, 7, 6) => base_slice.get(5).copied().unwrap_or(0.0),
        (6, 7, _) => 0.0,
        // 5.1 to >= 8ch: direct bed [FL, FR, FC, LFE, BL, BR], leave SL/SR/heights silent
        (6, hc, c) if hc >= 8 && c < 6 => base_slice.get(c).copied().unwrap_or(0.0),
        (6, hc, _) if hc >= 8 => 0.0,
        // 6.1 (FL, FR, FC, LFE, BC, SL, SR) to >= 8ch (7.1 0x63F): split BC -> BL/BR at -3dB
        (7, hc, 0..=3) if hc >= 8 => base_slice.get(out_channel).copied().unwrap_or(0.0),
        (7, hc, 4 | 5) if hc >= 8 => base_slice.get(4).copied().unwrap_or(0.0) * INV_SQRT2,
        (7, hc, 6) if hc >= 8 => base_slice.get(5).copied().unwrap_or(0.0),
        (7, hc, 7) if hc >= 8 => base_slice.get(6).copied().unwrap_or(0.0),
        (7, hc, _) if hc >= 8 => 0.0,
        // Stereo to mono
        (2, 1, 0) => {
            let l = base_slice.first().copied().unwrap_or(0.0);
            let r = base_slice.get(1).copied().unwrap_or(0.0);
            (l + r) * 0.5
        }
        // 3.0 / 2.1 (FL, FR, FC/LFE) to Stereo (L, R)
        (3, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (3, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        // 3.0 / 2.1 to Mono
        (3, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc) * 0.35
        }
        // Quad (FL, FR, BL, BR) to Stereo (L, R)
        (4, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let bl = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * bl) * INV_SQRT2
        }
        (4, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let br = base_slice.get(3).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * br) * INV_SQRT2
        }
        // Quad to Mono
        (4, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let bl = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(3).copied().unwrap_or(0.0);
            (fl + fr + INV_SQRT2 * (bl + br)) * 0.35
        }
        // 5.0 (FL, FR, FC, BL, BR) to 4.0 Quad (FL, FR, BL, BR)
        (5, 4, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (5, 4, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        (5, 4, 2) => base_slice.get(3).copied().unwrap_or(0.0),
        (5, 4, 3) => base_slice.get(4).copied().unwrap_or(0.0),
        // 5.0 to Stereo (L, R)
        (5, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(3).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * bl) * INV_SQRT2
        }
        (5, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(4).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * br) * INV_SQRT2
        }
        // 5.0 to Mono
        (5, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(3).copied().unwrap_or(0.0);
            let br = base_slice.get(4).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + INV_SQRT2 * (bl + br)) * 0.35
        }
        // 5.1 (FL, FR, FC, LFE, BL, BR) to 4.0 Quad (FL, FR, BL, BR) — exclude LFE
        (6, 4, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (6, 4, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        (6, 4, 2) => base_slice.get(4).copied().unwrap_or(0.0),
        (6, 4, 3) => base_slice.get(5).copied().unwrap_or(0.0),
        // 5.1 (FL, FR, FC, LFE, BL, BR) to Stereo (L, R) — ITU-R BS.775-3 (exclude LFE)
        (6, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * bl) * INV_SQRT2
        }
        (6, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * br) * INV_SQRT2
        }
        // 5.1 to Mono — ITU-R BS.775-3 (exclude LFE)
        (6, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + INV_SQRT2 * (bl + br)) * 0.35
        }
        // 6.1 (FL, FR, FC, LFE, BC, SL, SR) to 5.1 (FL, FR, FC, LFE, Ls, Rs)
        (7, 6, 0..=3) => base_slice.get(out_channel).copied().unwrap_or(0.0),
        (7, 6, 4) => {
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(5).copied().unwrap_or(0.0);
            (sl + INV_SQRT2 * bc) * INV_SQRT2
        }
        (7, 6, 5) => {
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sr = base_slice.get(6).copied().unwrap_or(0.0);
            (sr + INV_SQRT2 * bc) * INV_SQRT2
        }
        // 6.1 (FL, FR, FC, LFE, BC, SL, SR) to 4.0 Quad (FL, FR, Ls, Rs) — exclude LFE
        (7, 4, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (7, 4, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        (7, 4, 2) => {
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(5).copied().unwrap_or(0.0);
            (sl + INV_SQRT2 * bc) * INV_SQRT2
        }
        (7, 4, 3) => {
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sr = base_slice.get(6).copied().unwrap_or(0.0);
            (sr + INV_SQRT2 * bc) * INV_SQRT2
        }
        // 6.1 (FL, FR, FC, LFE, BC, SL, SR) to Stereo (L, R) — ITU-R BS.775-3 (exclude LFE)
        (7, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(5).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + 0.5 * bc + INV_SQRT2 * sl) * 0.65
        }
        (7, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sr = base_slice.get(6).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + 0.5 * bc + INV_SQRT2 * sr) * 0.65
        }
        // 6.1 to Mono — ITU-R BS.775-3 (exclude LFE)
        (7, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bc = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(6).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + INV_SQRT2 * (bc + sl + sr)) * 0.32
        }
        // 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR) to 5.1 (FL, FR, FC, LFE, SL/BL, SR/BR)
        (8, 6, 0..=3) => base_slice.get(out_channel).copied().unwrap_or(0.0),
        (8, 6, 4) => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            (bl + sl) * INV_SQRT2
        }
        (8, 6, 5) => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (br + sr) * INV_SQRT2
        }
        // 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR) to 4.0 Quad (FL, FR, Ls, Rs) — exclude LFE
        (8, 4, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc) * INV_SQRT2
        }
        (8, 4, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc) * INV_SQRT2
        }
        (8, 4, 2) => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            (bl + sl) * INV_SQRT2
        }
        (8, 4, 3) => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (br + sr) * INV_SQRT2
        }
        // 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR) to Stereo — ITU-R BS.775-3 (exclude LFE)
        (8, 2, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * bl + INV_SQRT2 * sl) * 0.65
        }
        (8, 2, 1) => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * br + INV_SQRT2 * sr) * 0.65
        }
        // 7.1 to Mono — ITU-R BS.775-3 (exclude LFE)
        (8, 1, 0) => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            (fl + fr + SQRT2 * fc + INV_SQRT2 * (bl + br + sl + sr)) * 0.30
        }
        // >8 channels (7.1.4, 7.1.2, 5.1.4, Ambisonics) to 7.1 (FL, FR, FC, LFE, BL, BR, SL, SR)
        (tc, 8, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let tfl = base_slice.get(8).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * tfl) * 0.85
        }
        (tc, 8, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let tfr = base_slice.get(9).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * tfr) * 0.85
        }
        (tc, 8, 2 | 3) if tc > 8 => base_slice.get(out_channel).copied().unwrap_or(0.0) * 0.85,
        (tc, 8, 4) if tc > 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (bl + 0.5 * tbl) * 0.85
        }
        (tc, 8, 5) if tc > 8 => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (br + 0.5 * tbr) * 0.85
        }
        (tc, 8, 6) if tc > 8 => {
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (sl + 0.5 * tbl) * 0.85
        }
        (tc, 8, 7) if tc > 8 => {
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (sr + 0.5 * tbr) * 0.85
        }
        // >8 channels (7.1.4, 7.1.2, 5.1.4, Ambisonics) to 6.1 (FL, FR, FC, LFE, BC, SL, SR)
        (tc, 7, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let tfl = base_slice.get(8).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * tfl) * 0.82
        }
        (tc, 7, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let tfr = base_slice.get(9).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * tfr) * 0.82
        }
        (tc, 7, 2 | 3) if tc > 8 => base_slice.get(out_channel).copied().unwrap_or(0.0) * 0.82,
        (tc, 7, 4) if tc > 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (bl + br + INV_SQRT2 * (tbl + tbr)) * 0.52
        }
        (tc, 7, 5) if tc > 8 => {
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (sl + 0.5 * tbl) * 0.82
        }
        (tc, 7, 6) if tc > 8 => {
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (sr + 0.5 * tbr) * 0.82
        }
        // >8 channels (7.1.4, 7.1.2, 5.1.4, Ambisonics) to 5.1 (FL, FR, FC, LFE, SL/BL, SR/BR)
        (tc, 6, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let tfl = base_slice.get(8).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * tfl) * 0.8
        }
        (tc, 6, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let tfr = base_slice.get(9).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * tfr) * 0.8
        }
        (tc, 6, 2 | 3) if tc > 8 => base_slice.get(out_channel).copied().unwrap_or(0.0) * 0.8,
        (tc, 6, 4) if tc > 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (bl + sl + INV_SQRT2 * tbl) * 0.65
        }
        (tc, 6, 5) if tc > 8 => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (br + sr + INV_SQRT2 * tbr) * 0.65
        }
        // >8 channels (7.1.4, 7.1.2, 5.1.4, Ambisonics) to 4.0 Quad (FL, FR, Ls, Rs) — exclude LFE
        (tc, 4, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let tfl = base_slice.get(8).copied().unwrap_or(0.0);
            (fl + INV_SQRT2 * fc + INV_SQRT2 * tfl) * 0.65
        }
        (tc, 4, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let tfr = base_slice.get(9).copied().unwrap_or(0.0);
            (fr + INV_SQRT2 * fc + INV_SQRT2 * tfr) * 0.65
        }
        (tc, 4, 2) if tc > 8 => {
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let tbl = base_slice.get(10).copied().unwrap_or(0.0);
            (bl + sl + INV_SQRT2 * tbl) * 0.65
        }
        (tc, 4, 3) if tc > 8 => {
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let tbr = base_slice.get(11).copied().unwrap_or(0.0);
            (br + sr + INV_SQRT2 * tbr) * 0.65
        }
        // >8 channels (7.1.4, Ambisonics, etc.) to Stereo — ITU-R BS.775-3 (exclude LFE)
        (tc, 2, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let bl = base_slice.get(4).copied().unwrap_or(0.0);
            let sl = base_slice.get(6).copied().unwrap_or(0.0);
            let mut sum = fl + INV_SQRT2 * (fc + bl + sl);
            for idx in (8..tc as usize).step_by(2) {
                sum += INV_SQRT2 * base_slice.get(idx).copied().unwrap_or(0.0);
            }
            sum * (2.0 / (tc as f32).sqrt()).min(0.6)
        }
        (tc, 2, 1) if tc > 8 => {
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let br = base_slice.get(5).copied().unwrap_or(0.0);
            let sr = base_slice.get(7).copied().unwrap_or(0.0);
            let mut sum = fr + INV_SQRT2 * (fc + br + sr);
            for idx in (9..tc as usize).step_by(2) {
                sum += INV_SQRT2 * base_slice.get(idx).copied().unwrap_or(0.0);
            }
            sum * (2.0 / (tc as f32).sqrt()).min(0.6)
        }
        // >8 channels to Mono — ITU-R BS.775-3 (exclude LFE at index 3)
        (tc, 1, 0) if tc > 8 => {
            let fl = base_slice.first().copied().unwrap_or(0.0);
            let fr = base_slice.get(1).copied().unwrap_or(0.0);
            let fc = base_slice.get(2).copied().unwrap_or(0.0);
            let mut sum = fl + fr + SQRT2 * fc;
            for idx in 4..tc as usize {
                sum += INV_SQRT2 * base_slice.get(idx).copied().unwrap_or(0.0);
            }
            sum * (1.0 / (tc as f32).sqrt()).min(0.30)
        }
        // General fallback: if channel index exists in source, take it
        (tc, _, c) if c < tc as usize => base_slice.get(c).copied().unwrap_or(0.0),
        _ => 0.0,
    }
}

struct SymphoniaSource {
    format: Box<dyn FormatReader>,
    active_tracks: std::collections::HashMap<u32, ActiveSymphoniaTrack>,
    primary_track_id: u32,
    time_base: f64,
    current_time: f64,
    duration: f64,
    channels: u16,
    channel_vus: Vec<f32>,
    artist: String,
    ext_type: String,
    intrinsic_sample_rate: Option<u32>,
    output_sample_rate: u32,
    video_info: Option<String>,
    audio_tracks: Vec<crate::state::AudioTrackInfo>,
    selected_track_idx: usize,
    has_video: bool,
    bitrate: Option<u32>,
    is_full_mix_guide: bool,
}

impl AudioSource for SymphoniaSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let target_rate = if sample_rate > 0 {
            sample_rate
        } else {
            self.output_sample_rate.max(44100)
        };
        if target_rate != self.output_sample_rate {
            self.output_sample_rate = target_rate;
            for track in self.active_tracks.values_mut() {
                track.samples.clear();
                track.buf_pos = 0;
                if let Some(ref mut rs) = track.sinc_resampler {
                    rs.reset();
                }
            }
        }
        let mut frames_written = 0;
        let frames_needed = output.len() / hardware_channels;
        self.channel_vus.fill(0.0);

        let is_2track = self.audio_tracks.len() == 2;

        // Dynamic energy-cancellation check for 2-track stereo audio (instrumental + full mix vocal guide)
        if !self.is_full_mix_guide && is_2track {
            let t0_opt = self.active_tracks.values().find(|t| t.track_idx == 0);
            let t1_opt = self.active_tracks.values().find(|t| t.track_idx == 1);
            if let (Some(t0), Some(t1)) = (t0_opt, t1_opt) {
                let ch0 = t0.channels as usize;
                let ch1 = t1.channels as usize;
                if ch0 == ch1 && ch0 <= 2 {
                    let avail0 = (t0.samples.len().saturating_sub(t0.buf_pos)) / ch0.max(1);
                    let avail1 = (t1.samples.len().saturating_sub(t1.buf_pos)) / ch1.max(1);
                    let check_frames = avail0.min(avail1).min(1024);
                    if check_frames >= 512 {
                        let mut e0 = 0.0f32;
                        let mut e1 = 0.0f32;
                        let mut ediff = 0.0f32;
                        for i in 0..check_frames {
                            let s0 = t0.samples.get(t0.buf_pos + i * ch0).copied().unwrap_or(0.0);
                            let s1 = t1.samples.get(t1.buf_pos + i * ch1).copied().unwrap_or(0.0);
                            e0 += s0 * s0;
                            e1 += s1 * s1;
                            ediff += (s1 - s0) * (s1 - s0);
                        }
                        let min_energy = 0.02 * (check_frames as f32);
                        if e0 > min_energy
                            && e1 > min_energy
                            && ediff > 0.005 * e1
                            && ediff < 0.30 * e1
                        {
                            self.is_full_mix_guide = true;
                            eprintln!(
                                "[RustTracker Audio] Detected Full Mix vocal guide stem - enabling phase cancellation"
                            );
                            #[cfg(target_os = "android")]
                            crate::android::log_android(
                                3,
                                "[RustTracker Audio] Detected Full Mix vocal guide stem - enabling phase cancellation",
                            );
                        }
                    }
                }
            }
        }

        let need_t0 = is_2track && (self.is_full_mix_guide || self.current_time < 3.0);

        while frames_written < frames_needed {
            let mut need_more = false;
            for track in self.active_tracks.values().filter(|t| {
                t.enabled || t.track_id == self.primary_track_id || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.samples.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                if avail < (frames_needed - frames_written) {
                    need_more = true;
                    break;
                }
            }

            if need_more {
                for _ in 0..512 {
                    let all_satisfied = self
                        .active_tracks
                        .values()
                        .filter(|t| {
                            t.enabled
                                || t.track_id == self.primary_track_id
                                || (need_t0 && t.track_idx == 0)
                        })
                        .all(|t| {
                            let avail = (t.samples.len().saturating_sub(t.buf_pos))
                                / t.channels.max(1) as usize;
                            avail >= (frames_needed - frames_written)
                        });
                    if all_satisfied {
                        break;
                    }

                    match self.format.next_packet() {
                        Ok(Some(packet)) => {
                            let pkt_track_id = packet.track_id;
                            if let Some(track) = self.active_tracks.get_mut(&pkt_track_id)
                                && (track.enabled
                                    || track.track_id == self.primary_track_id
                                    || (need_t0 && track.track_idx == 0))
                                && let Ok(decoded) = track.decoder.decode(&packet)
                            {
                                let in_rate = decoded.spec().rate().max(1);
                                track.sample_buf.clear();
                                decoded.copy_to_vec_interleaved(&mut track.sample_buf);

                                if track.buf_pos > 4096 {
                                    track.samples.drain(0..track.buf_pos);
                                    track.buf_pos = 0;
                                }
                                track.sample_rate = in_rate;
                                let out_rate = self.output_sample_rate.max(1);
                                let src = track.sample_buf.as_slice();
                                let ch = track.channels.max(1) as usize;
                                if in_rate == out_rate {
                                    track.samples.extend_from_slice(src);
                                } else {
                                    let needs_new = !track
                                        .sinc_resampler
                                        .as_ref()
                                        .is_some_and(|rs| rs.matches(in_rate, out_rate, ch));
                                    if needs_new {
                                        track.sinc_resampler =
                                            StreamingSincResampler::new(in_rate, out_rate, ch).ok();
                                    }
                                    if let Some(ref mut rs) = track.sinc_resampler {
                                        rs.push_interleaved(src, &mut track.samples);
                                    } else {
                                        track.samples.extend_from_slice(src);
                                    }
                                }
                            }
                        }
                        Ok(None) | Err(_) => {
                            for track in self.active_tracks.values_mut() {
                                if let Some(ref mut rs) = track.sinc_resampler {
                                    rs.flush(&mut track.samples);
                                }
                            }
                            break;
                        }
                    }
                }
            }

            let mut min_available = usize::MAX;
            for track in self.active_tracks.values().filter(|t| {
                t.enabled || t.track_id == self.primary_track_id || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.samples.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                min_available = min_available.min(avail);
            }

            if min_available == 0 || min_available == usize::MAX {
                if let Some(primary) = self.active_tracks.get(&self.primary_track_id) {
                    let avail = (primary.samples.len().saturating_sub(primary.buf_pos))
                        / primary.channels.max(1) as usize;
                    if avail == 0 {
                        break;
                    }
                    min_available = avail;
                } else {
                    break;
                }
            }

            let frames_to_copy = min_available.min(frames_needed - frames_written);
            if frames_to_copy == 0 {
                break;
            }

            let headroom = calculate_mix_headroom(
                self.active_tracks
                    .values()
                    .filter(|t| t.enabled)
                    .map(|t| t.volume),
            );

            let is_full_mix = self.is_full_mix_guide && is_2track;
            let (t0_info, t1_info) = if is_full_mix {
                let mut t0 = None;
                let mut t1 = None;
                for t in self.active_tracks.values() {
                    if t.track_idx == 0 {
                        t0 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    } else if t.track_idx == 1 {
                        t1 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    }
                }
                (t0, t1)
            } else {
                (None, None)
            };

            if is_full_mix && let (Some(t0), Some(t1)) = (t0_info, t1_info) {
                let v0 = if t0.3 { t0.2 } else { 0.0 };
                let v1 = if t1.3 { t1.2 } else { 0.0 };
                let vocal_headroom = 1.0 / ((v0 - v1).abs() + v1.abs()).max(1.0);

                let t0_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 0)
                    .unwrap()
                    .samples;
                let t1_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 1)
                    .unwrap()
                    .samples;

                for f in 0..frames_to_copy {
                    let out_base = (frames_written + f) * hardware_channels;
                    let base_idx_0 = t0.0 + f * t0.1 as usize;
                    let base_idx_1 = t1.0 + f * t1.1 as usize;
                    let ch_slice_0 = if base_idx_0 < t0_samples.len() {
                        &t0_samples[base_idx_0..]
                    } else {
                        &[]
                    };
                    let ch_slice_1 = if base_idx_1 < t1_samples.len() {
                        &t1_samples[base_idx_1..]
                    } else {
                        &[]
                    };

                    for c in 0..hardware_channels {
                        let s0 = get_downmixed_sample(t0.1, ch_slice_0, hardware_channels, c);
                        let s1 = get_downmixed_sample(t1.1, ch_slice_1, hardware_channels, c);
                        // Extracted Vocals V = s1 - s0
                        // Desired output: v0 * I + v1 * V = (v0 - v1) * s0 + v1 * s1
                        let sum_val = ((v0 - v1) * s0 + v1 * s1) * vocal_headroom;
                        let final_val = sum_val.clamp(-1.0, 1.0);
                        output[out_base + c] = final_val;

                        if c < self.channel_vus.len() {
                            self.channel_vus[c] = self.channel_vus[c].max(final_val.abs());
                        }
                    }
                }
            } else {
                for f in 0..frames_to_copy {
                    let out_base = (frames_written + f) * hardware_channels;

                    // Update true post-mix channel VUs across all source channels with headroom applied
                    for ch in 0..self.channel_vus.len() {
                        let mut ch_sum = 0.0f32;
                        for track in self.active_tracks.values() {
                            if track.enabled && track.volume > 0.0 && ch < track.channels as usize {
                                let base_idx = track.buf_pos + f * track.channels as usize;
                                let s = track.samples.get(base_idx + ch).copied().unwrap_or(0.0);
                                ch_sum += s * track.volume;
                            }
                        }
                        self.channel_vus[ch] =
                            self.channel_vus[ch].max((ch_sum * headroom).abs().min(1.0));
                    }

                    for c in 0..hardware_channels {
                        let mut sum_val = 0.0f32;
                        for track in self.active_tracks.values() {
                            if track.enabled && track.volume > 0.0 {
                                let base_idx = track.buf_pos + f * track.channels as usize;
                                let ch_slice = if base_idx < track.samples.len() {
                                    &track.samples[base_idx..]
                                } else {
                                    &[]
                                };
                                let s_val = get_downmixed_sample(
                                    track.channels,
                                    ch_slice,
                                    hardware_channels,
                                    c,
                                );
                                sum_val += s_val * track.volume;
                            }
                        }
                        let final_val = (sum_val * headroom).clamp(-1.0, 1.0);
                        output[out_base + c] = final_val;
                    }
                }
            }

            for track in self.active_tracks.values_mut().filter(|t| {
                t.enabled || t.track_id == self.primary_track_id || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.samples.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                let consume = frames_to_copy.min(avail);
                track.buf_pos += consume * track.channels as usize;
                track.current_time += consume as f64 / self.output_sample_rate.max(1) as f64;
            }
            if let Some(primary) = self.active_tracks.get(&self.primary_track_id) {
                self.current_time = primary.current_time;
            }
            frames_written += frames_to_copy;
        }

        frames_written
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }

    fn set_position_seconds(&mut self, pos: f64) {
        let ts = Timestamp::new((pos / self.time_base) as i64);
        let seek_res = self.format.seek(
            symphonia::core::formats::SeekMode::Coarse,
            symphonia::core::formats::SeekTo::Timestamp {
                ts,
                track_id: self.primary_track_id,
            },
        );

        if seek_res.is_ok() {
            for track in self.active_tracks.values_mut() {
                track.decoder.reset();
                track.current_time = pos;
                track.samples.clear();
                track.buf_pos = 0;
                if let Some(ref mut rs) = track.sinc_resampler {
                    rs.reset();
                }
            }
            self.current_time = pos;
        } else {
            let seek_res_start = self.format.seek(
                symphonia::core::formats::SeekMode::Coarse,
                symphonia::core::formats::SeekTo::Timestamp {
                    ts: Timestamp::ZERO,
                    track_id: self.primary_track_id,
                },
            );
            if seek_res_start.is_ok() {
                for track in self.active_tracks.values_mut() {
                    track.decoder.reset();
                    track.current_time = pos;
                    track.samples.clear();
                    track.buf_pos = 0;
                    if let Some(ref mut rs) = track.sinc_resampler {
                        rs.reset();
                    }
                }
                self.current_time = pos;
            }
        }
    }

    fn get_num_channels(&mut self) -> i32 {
        self.channels as i32
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        self.channel_vus
            .get(channel as usize)
            .cloned()
            .unwrap_or(0.0)
    }
    fn get_type(&mut self) -> String {
        self.ext_type.clone()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        self.intrinsic_sample_rate
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn get_tracker_channels(&mut self) -> Option<i32> {
        None
    }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn get_current_row_string(&mut self) -> String {
        String::new()
    }
    fn get_artist(&mut self) -> String {
        self.artist.clone()
    }
    fn get_video_info(&mut self) -> Option<String> {
        self.video_info.clone()
    }

    fn get_audio_tracks(&self) -> Vec<crate::state::AudioTrackInfo> {
        self.audio_tracks.clone()
    }
    fn get_selected_audio_track(&self) -> usize {
        self.selected_track_idx
    }

    fn get_active_audio_tracks(&self) -> Vec<usize> {
        let mut list: Vec<usize> = self
            .active_tracks
            .values()
            .filter(|t| t.enabled)
            .map(|t| t.track_idx)
            .collect();
        list.sort();
        list.dedup();
        if list.is_empty() {
            vec![self.selected_track_idx]
        } else {
            list
        }
    }

    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }

        let track_map: std::collections::HashMap<usize, f32> = tracks.iter().copied().collect();
        for track in self.active_tracks.values_mut() {
            if let Some(&vol) = track_map.get(&track.track_idx) {
                track.enabled = true;
                track.volume = vol;
            } else {
                track.enabled = false;
                track.volume = 0.0;
            }
        }

        if let Some(&(first_idx, _)) = tracks.first()
            && first_idx < self.audio_tracks.len()
        {
            self.selected_track_idx = first_idx;
            self.primary_track_id = self.audio_tracks[first_idx].id as u32;
            let container = self.ext_type.split('/').nth(1).map(|s| s.to_string());
            if let Some(container) = container {
                self.ext_type = format!("{}/{}", self.audio_tracks[first_idx].codec, container);
            } else {
                self.ext_type = self.audio_tracks[first_idx].codec.clone();
            }
            self.channels = self.audio_tracks[first_idx].channels;
            self.channel_vus = vec![0.0; self.channels as usize];
            self.intrinsic_sample_rate = Some(self.audio_tracks[first_idx].sample_rate);
            if let Some(t) = self.active_tracks.get(&self.primary_track_id) {
                self.time_base = t.time_base;
            }
        }
        Ok(())
    }

    fn select_audio_track(&mut self, track_idx: usize) -> Result<()> {
        self.set_active_audio_tracks(&[(track_idx, 1.0)])
    }

    fn has_video_stream(&self) -> bool {
        self.has_video
    }

    fn get_bitrate(&mut self) -> Option<u32> {
        self.bitrate
    }
}

// ---------------------------------------------------------
// FFmpeg Native Decoder (including AOMedia IAMF / Eclipsa Audio)
// ---------------------------------------------------------
#[derive(Clone, Debug)]
pub struct IamfSubstreamChannelMap {
    pub stream_index: usize,
    pub substream_channel: usize,
    /// `(target_speaker_idx, linear_gain)` in canonical speaker order:
    /// `[0:FL, 1:FR, 2:FC, 3:LFE, 4:BL, 5:BR, 6:SL, 7:SR, 8:TFL, 9:TFR, 10:TBL, 11:TBR]`
    pub targets: Vec<(usize, f32)>,
}

#[derive(Clone, Debug)]
pub struct IamfTrackGroup {
    pub substream_indices: Vec<usize>,
    pub reconstructed_channels: u16,
    pub layout_label: String,
    pub annotation: Option<String>,
    pub language: Option<String>,
    pub channel_mappings: Vec<IamfSubstreamChannelMap>,
}

#[repr(C)]
struct FfiIamfLayer {
    av_class: *const ffmpeg_next::ffi::AVClass,
    ch_layout: ffmpeg_next::ffi::AVChannelLayout,
    flags: std::ffi::c_uint,
    output_gain_flags: std::ffi::c_uint,
    output_gain: ffmpeg_next::ffi::AVRational,
    ambisonics_mode: std::ffi::c_uint,
    demixing_matrix: *mut ffmpeg_next::ffi::AVRational,
    nb_demixing_matrix: std::ffi::c_uint,
}

#[repr(C)]
struct FfiIamfAudioElement {
    av_class: *const ffmpeg_next::ffi::AVClass,
    layers: *mut *mut FfiIamfLayer,
    nb_layers: std::ffi::c_uint,
    demixing_info: *mut std::ffi::c_void,
    recon_gain_info: *mut std::ffi::c_void,
    audio_element_type: std::ffi::c_uint,
    default_w: std::ffi::c_uint,
}

#[repr(C)]
struct FfiIamfSubmixElement {
    av_class: *const ffmpeg_next::ffi::AVClass,
    audio_element_id: std::ffi::c_uint,
    element_mix_config: *mut std::ffi::c_void,
    default_mix_gain: ffmpeg_next::ffi::AVRational,
    headphones_rendering_mode: std::ffi::c_uint,
    annotations: *mut ffmpeg_next::ffi::AVDictionary,
}

#[repr(C)]
struct FfiIamfSubmix {
    av_class: *const ffmpeg_next::ffi::AVClass,
    elements: *mut *mut FfiIamfSubmixElement,
    nb_elements: std::ffi::c_uint,
    layouts: *mut *mut std::ffi::c_void,
    nb_layouts: std::ffi::c_uint,
    output_mix_config: *mut std::ffi::c_void,
    default_mix_gain: ffmpeg_next::ffi::AVRational,
}

#[repr(C)]
struct FfiIamfMixPresentation {
    av_class: *const ffmpeg_next::ffi::AVClass,
    submixes: *mut *mut FfiIamfSubmix,
    nb_submixes: std::ffi::c_uint,
    annotations: *mut ffmpeg_next::ffi::AVDictionary,
}

fn iamf_rational_db_to_linear(r: ffmpeg_next::ffi::AVRational) -> f32 {
    if r.den == 0 || r.num == 0 {
        return 1.0;
    }
    let db = (r.num as f32 / r.den as f32).clamp(-60.0, 24.0);
    10.0f32.powf(db / 20.0)
}

unsafe fn extract_first_av_dict_entry(
    dict: *const ffmpeg_next::ffi::AVDictionary,
) -> (Option<String>, Option<String>) {
    if dict.is_null() {
        return (None, None);
    }
    let entry = unsafe {
        ffmpeg_next::ffi::av_dict_get(
            dict,
            c"".as_ptr(),
            std::ptr::null(),
            ffmpeg_next::ffi::AV_DICT_IGNORE_SUFFIX,
        )
    };
    if entry.is_null() {
        return (None, None);
    }
    let key = if !unsafe { (*entry).key }.is_null() {
        unsafe { std::ffi::CStr::from_ptr((*entry).key) }
            .to_str()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.to_lowercase() != "und")
    } else {
        None
    };
    let val = if !unsafe { (*entry).value }.is_null() {
        unsafe { std::ffi::CStr::from_ptr((*entry).value) }
            .to_str()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "default_mix_presentation")
    } else {
        None
    };
    (key, val)
}

pub fn map_iamf_av_channels_to_canonical(
    av_channels: &[i32],
    substreams: &[(usize, u16)],
) -> (u16, String, Vec<IamfSubstreamChannelMap>) {
    use ffmpeg_next::ffi::AVChannel::*;
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const AV_CHAN_BINAURAL_LEFT_ID: i32 = 61;
    const AV_CHAN_BINAURAL_RIGHT_ID: i32 = 62;

    let has_height = av_channels.iter().any(|&c| {
        c == AV_CHAN_TOP_CENTER as i32
            || c == AV_CHAN_TOP_FRONT_LEFT as i32
            || c == AV_CHAN_TOP_FRONT_CENTER as i32
            || c == AV_CHAN_TOP_FRONT_RIGHT as i32
            || c == AV_CHAN_TOP_BACK_LEFT as i32
            || c == AV_CHAN_TOP_BACK_CENTER as i32
            || c == AV_CHAN_TOP_BACK_RIGHT as i32
            || c == AV_CHAN_TOP_SIDE_LEFT as i32
            || c == AV_CHAN_TOP_SIDE_RIGHT as i32
            || c == AV_CHAN_TOP_SURROUND_LEFT as i32
            || c == AV_CHAN_TOP_SURROUND_RIGHT as i32
    });
    let has_back = av_channels
        .iter()
        .any(|&c| c == AV_CHAN_BACK_LEFT as i32 || c == AV_CHAN_BACK_RIGHT as i32);
    let has_side = av_channels.iter().any(|&c| {
        c == AV_CHAN_SIDE_LEFT as i32
            || c == AV_CHAN_SIDE_RIGHT as i32
            || c == AV_CHAN_SIDE_SURROUND_LEFT as i32
            || c == AV_CHAN_SIDE_SURROUND_RIGHT as i32
            || c == AV_CHAN_SURROUND_DIRECT_LEFT as i32
            || c == AV_CHAN_SURROUND_DIRECT_RIGHT as i32
    });
    let has_top_back = av_channels
        .iter()
        .any(|&c| c == AV_CHAN_TOP_BACK_LEFT as i32 || c == AV_CHAN_TOP_BACK_RIGHT as i32);

    let nb_ch = av_channels.len();
    let (recon_channels, layout_label) = if has_height {
        let lbl = match nb_ch {
            12 => "7.1.4".to_string(),
            10 if has_back && has_side => "7.1.2".to_string(),
            10 => "5.1.4".to_string(),
            8 => "5.1.2".to_string(),
            6 => "3.1.2".to_string(),
            14 => "9.1.4".to_string(),
            16 => "9.1.6".to_string(),
            n => format!("{} ch", n),
        };
        (12u16, lbl)
    } else if nb_ch >= 7 {
        (
            8u16,
            if nb_ch == 8 {
                "7.1".to_string()
            } else {
                "6.1".to_string()
            },
        )
    } else if nb_ch >= 5 {
        (
            6u16,
            if nb_ch == 6 {
                "5.1".to_string()
            } else {
                "5.0".to_string()
            },
        )
    } else if nb_ch == 4 {
        (4u16, "4.0".to_string())
    } else if nb_ch == 3 {
        (3u16, "3.0".to_string())
    } else if nb_ch == 2 {
        (2u16, "Stereo".to_string())
    } else {
        (1u16, "Mono".to_string())
    };

    let mut mappings = Vec::with_capacity(nb_ch);
    let mut flat_idx = 0usize;
    for &(stream_index, sub_ch_count) in substreams {
        for sub_ch in 0..(sub_ch_count as usize) {
            let chan_id = av_channels
                .get(flat_idx)
                .copied()
                .unwrap_or(AV_CHAN_NONE as i32);
            flat_idx += 1;

            let targets: Vec<(usize, f32)> = if chan_id == AV_CHAN_FRONT_LEFT as i32
                || chan_id == AV_CHAN_STEREO_LEFT as i32
                || chan_id == AV_CHAN_BINAURAL_LEFT_ID
                || chan_id == AV_CHAN_WIDE_LEFT as i32
            {
                vec![(0, 1.0)]
            } else if chan_id == AV_CHAN_FRONT_RIGHT as i32
                || chan_id == AV_CHAN_STEREO_RIGHT as i32
                || chan_id == AV_CHAN_BINAURAL_RIGHT_ID
                || chan_id == AV_CHAN_WIDE_RIGHT as i32
            {
                vec![(1, 1.0)]
            } else if chan_id == AV_CHAN_FRONT_CENTER as i32 {
                if recon_channels == 1 {
                    vec![(0, 1.0)]
                } else if recon_channels == 2 {
                    vec![(0, INV_SQRT2), (1, INV_SQRT2)]
                } else {
                    vec![(2, 1.0)]
                }
            } else if chan_id == AV_CHAN_LOW_FREQUENCY as i32
                || chan_id == AV_CHAN_LOW_FREQUENCY_2 as i32
            {
                if recon_channels >= 4 {
                    vec![(3, 1.0)]
                } else {
                    vec![(0, 0.5), (1, 0.5)]
                }
            } else if chan_id == AV_CHAN_BACK_LEFT as i32 {
                if recon_channels == 6 {
                    vec![(4, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_side {
                        vec![(4, INV_SQRT2), (6, INV_SQRT2)]
                    } else {
                        vec![(4, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(2, 1.0)]
                } else {
                    vec![(0, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_BACK_RIGHT as i32 {
                if recon_channels == 6 {
                    vec![(5, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_side {
                        vec![(5, INV_SQRT2), (7, INV_SQRT2)]
                    } else {
                        vec![(5, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(3, 1.0)]
                } else {
                    vec![(1, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_SIDE_LEFT as i32
                || chan_id == AV_CHAN_SIDE_SURROUND_LEFT as i32
                || chan_id == AV_CHAN_SURROUND_DIRECT_LEFT as i32
            {
                if recon_channels == 6 {
                    vec![(4, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_back {
                        vec![(4, INV_SQRT2), (6, INV_SQRT2)]
                    } else {
                        vec![(6, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(2, 1.0)]
                } else {
                    vec![(0, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_SIDE_RIGHT as i32
                || chan_id == AV_CHAN_SIDE_SURROUND_RIGHT as i32
                || chan_id == AV_CHAN_SURROUND_DIRECT_RIGHT as i32
            {
                if recon_channels == 6 {
                    vec![(5, 1.0)]
                } else if recon_channels >= 8 {
                    if !has_back {
                        vec![(5, INV_SQRT2), (7, INV_SQRT2)]
                    } else {
                        vec![(7, 1.0)]
                    }
                } else if recon_channels == 4 {
                    vec![(3, 1.0)]
                } else {
                    vec![(1, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_BACK_CENTER as i32 {
                if recon_channels >= 6 {
                    vec![(4, INV_SQRT2), (5, INV_SQRT2)]
                } else {
                    vec![(0, 0.5), (1, 0.5)]
                }
            } else if chan_id == AV_CHAN_TOP_FRONT_LEFT as i32
                || chan_id == AV_CHAN_TOP_SIDE_LEFT as i32
                || chan_id == AV_CHAN_TOP_SURROUND_LEFT as i32
            {
                if recon_channels >= 12 {
                    if !has_top_back {
                        vec![(8, INV_SQRT2), (10, INV_SQRT2)]
                    } else {
                        vec![(8, 1.0)]
                    }
                } else {
                    vec![(0, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_FRONT_RIGHT as i32
                || chan_id == AV_CHAN_TOP_SIDE_RIGHT as i32
                || chan_id == AV_CHAN_TOP_SURROUND_RIGHT as i32
            {
                if recon_channels >= 12 {
                    if !has_top_back {
                        vec![(9, INV_SQRT2), (11, INV_SQRT2)]
                    } else {
                        vec![(9, 1.0)]
                    }
                } else {
                    vec![(1, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_BACK_LEFT as i32 {
                if recon_channels >= 12 {
                    vec![(10, 1.0)]
                } else {
                    vec![(4, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_BACK_RIGHT as i32 {
                if recon_channels >= 12 {
                    vec![(11, 1.0)]
                } else {
                    vec![(5, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_FRONT_CENTER as i32 {
                if recon_channels >= 12 {
                    vec![(8, INV_SQRT2), (9, INV_SQRT2)]
                } else {
                    vec![(2, INV_SQRT2)]
                }
            } else if chan_id == AV_CHAN_TOP_BACK_CENTER as i32 {
                if recon_channels >= 12 {
                    vec![(10, INV_SQRT2), (11, INV_SQRT2)]
                } else {
                    vec![(4, 0.5), (5, 0.5)]
                }
            } else if chan_id == AV_CHAN_TOP_CENTER as i32 {
                if recon_channels >= 12 {
                    vec![(8, 0.5), (9, 0.5), (10, 0.5), (11, 0.5)]
                } else {
                    vec![(0, 0.5), (1, 0.5)]
                }
            } else if chan_id == AV_CHAN_FRONT_LEFT_OF_CENTER as i32 {
                if recon_channels >= 3 {
                    vec![(0, 0.8), (2, 0.5)]
                } else {
                    vec![(0, 1.0)]
                }
            } else if chan_id == AV_CHAN_FRONT_RIGHT_OF_CENTER as i32 {
                if recon_channels >= 3 {
                    vec![(1, 0.8), (2, 0.5)]
                } else {
                    vec![(1, 1.0)]
                }
            } else {
                let fallback_ch = (flat_idx - 1).min(recon_channels.saturating_sub(1) as usize);
                vec![(fallback_ch, 1.0)]
            };

            mappings.push(IamfSubstreamChannelMap {
                stream_index,
                substream_channel: sub_ch,
                targets,
            });
        }
    }

    (recon_channels, layout_label, mappings)
}

pub(in crate::audio) fn map_iamf_ambisonics_to_canonical(
    substreams: &[(usize, u16)],
    nb_acn_channels: usize,
) -> (u16, String, Vec<IamfSubstreamChannelMap>) {
    // ITU-R BS.2051-3 7.1.4 speaker unit vectors (x=cos(az)*cos(el), y=sin(az)*cos(el), z=sin(el))
    // in canonical speaker order [0:FL, 1:FR, 2:FC, 3:LFE, 4:BL, 5:BR, 6:SL, 7:SR, 8:TFL, 9:TFR, 10:TBL, 11:TBR]
    const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const SPEAKER_DIRS: [(f32, f32, f32); 12] = [
        (0.8660, 0.5000, 0.0000),      // 0: FL (+30, 0)
        (0.8660, -0.5000, 0.0000),     // 1: FR (-30, 0)
        (1.0000, 0.0000, 0.0000),      // 2: FC (0, 0)
        (0.0000, 0.0000, 0.0000),      // 3: LFE (omni W only)
        (-INV_SQRT2, INV_SQRT2, 0.0),  // 4: BL (+135, 0)
        (-INV_SQRT2, -INV_SQRT2, 0.0), // 5: BR (-135, 0)
        (0.0000, 1.0000, 0.0000),      // 6: SL (+90, 0)
        (0.0000, -1.0000, 0.0000),     // 7: SR (-90, 0)
        (0.5000, 0.5000, INV_SQRT2),   // 8: TFL (+45, +45)
        (0.5000, -0.5000, INV_SQRT2),  // 9: TFR (-45, +45)
        (-0.5000, 0.5000, INV_SQRT2),  // 10: TBL (+135, +45)
        (-0.5000, -0.5000, INV_SQRT2), // 11: TBR (-135, +45)
    ];

    let mut mappings = Vec::new();
    let mut acn_idx = 0usize;
    for &(stream_index, sub_ch_count) in substreams {
        for sub_ch in 0..(sub_ch_count as usize) {
            let mut targets = Vec::new();
            match acn_idx {
                // ACN 0 = W (omnidirectional)
                0 => {
                    for s in 0..12 {
                        let w = if s == 3 { 0.25 } else { 0.35 };
                        targets.push((s, w));
                    }
                }
                // ACN 1 = Y (Left-Right dipole)
                1 => {
                    for (s, &(_, y, _)) in SPEAKER_DIRS.iter().enumerate() {
                        if s != 3 && y.abs() > 0.01 {
                            targets.push((s, 0.35 * y));
                        }
                    }
                }
                // ACN 2 = Z (Up-Down dipole)
                2 => {
                    for (s, &(_, _, z)) in SPEAKER_DIRS.iter().enumerate() {
                        if s != 3 && z.abs() > 0.01 {
                            targets.push((s, 0.35 * z));
                        }
                    }
                }
                // ACN 3 = X (Front-Back dipole)
                3 => {
                    for (s, &(x, _, _)) in SPEAKER_DIRS.iter().enumerate() {
                        if s != 3 && x.abs() > 0.01 {
                            targets.push((s, 0.35 * x));
                        }
                    }
                }
                _ => {}
            }
            mappings.push(IamfSubstreamChannelMap {
                stream_index,
                substream_channel: sub_ch,
                targets,
            });
            acn_idx += 1;
        }
    }

    let order_label = match nb_acn_channels {
        4 => "FOA 7.1.4".to_string(),
        9 => "SOA 7.1.4".to_string(),
        16 => "TOA 7.1.4".to_string(),
        n => format!("Ambisonics {}ch", n),
    };
    (12, order_label, mappings)
}

fn probe_iamf_stream_groups(ictx: &ffmpeg_next::format::context::Input) -> Vec<IamfTrackGroup> {
    let mut result = Vec::new();

    unsafe {
        let fmt_ptr = ictx.as_ptr();
        if fmt_ptr.is_null() {
            return result;
        }
        let nb_groups = (*fmt_ptr).nb_stream_groups as usize;
        let groups_ptr = (*fmt_ptr).stream_groups;
        if nb_groups == 0 || groups_ptr.is_null() {
            return result;
        }

        let mut audio_elements: Vec<(i64, IamfTrackGroup)> = Vec::new();

        // 1. Parse all IAMF Audio Element stream groups
        for g_idx in 0..nb_groups {
            let grp = *groups_ptr.add(g_idx);
            if grp.is_null() {
                continue;
            }
            if (*grp).type_
                != ffmpeg_next::ffi::AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_IAMF_AUDIO_ELEMENT
            {
                continue;
            }
            let elem_ptr = (*grp).params.iamf_audio_element as *const FfiIamfAudioElement;
            if elem_ptr.is_null() || (*elem_ptr).nb_layers == 0 || (*elem_ptr).layers.is_null() {
                continue;
            }

            let nb_streams = (*grp).nb_streams as usize;
            if nb_streams == 0 || (*grp).streams.is_null() {
                continue;
            }

            let mut substreams = Vec::with_capacity(nb_streams);
            let mut substream_indices = Vec::with_capacity(nb_streams);
            for s in 0..nb_streams {
                let st_ptr = *(*grp).streams.add(s);
                if st_ptr.is_null() || (*st_ptr).codecpar.is_null() {
                    continue;
                }
                let s_idx = (*st_ptr).index as usize;
                let ch = (*(*st_ptr).codecpar).ch_layout.nb_channels.max(1) as u16;
                substreams.push((s_idx, ch));
                if !substream_indices.contains(&s_idx) {
                    substream_indices.push(s_idx);
                }
            }
            if substreams.is_empty() {
                continue;
            }

            let nb_layers = (*elem_ptr).nb_layers as usize;
            let final_layer = *(*elem_ptr).layers.add(nb_layers - 1);
            if final_layer.is_null() {
                continue;
            }

            let layout_ptr = &(*final_layer).ch_layout;
            let nb_layout_ch = layout_ptr.nb_channels.max(1) as usize;
            let layer_gain = if (*final_layer).output_gain_flags != 0 {
                iamf_rational_db_to_linear((*final_layer).output_gain)
            } else {
                1.0
            };

            let (recon_channels, layout_label, mut mappings) =
                if (*elem_ptr).audio_element_type == 1 {
                    map_iamf_ambisonics_to_canonical(&substreams, nb_layout_ch)
                } else {
                    let mut av_channels = Vec::with_capacity(nb_layout_ch);
                    for c_idx in 0..nb_layout_ch {
                        let ch_enum = ffmpeg_next::ffi::av_channel_layout_channel_from_index(
                            layout_ptr,
                            c_idx as std::ffi::c_uint,
                        );
                        av_channels.push(ch_enum as i32);
                    }
                    map_iamf_av_channels_to_canonical(&av_channels, &substreams)
                };

            if (layer_gain - 1.0).abs() > 1e-4 {
                for m in &mut mappings {
                    for t in &mut m.targets {
                        t.1 *= layer_gain;
                    }
                }
            }

            audio_elements.push((
                (*grp).id,
                IamfTrackGroup {
                    substream_indices,
                    reconstructed_channels: recon_channels,
                    layout_label,
                    annotation: None,
                    language: None,
                    channel_mappings: mappings,
                },
            ));
        }

        if audio_elements.is_empty() {
            return result;
        }

        let mut used_element_ids = std::collections::HashSet::new();

        // 2. Parse IAMF Mix Presentation stream groups referencing the Audio Elements
        for g_idx in 0..nb_groups {
            let grp = *groups_ptr.add(g_idx);
            if grp.is_null() {
                continue;
            }
            if (*grp).type_
                != ffmpeg_next::ffi::AVStreamGroupParamsType::AV_STREAM_GROUP_PARAMS_IAMF_MIX_PRESENTATION
            {
                continue;
            }
            let mix_ptr = (*grp).params.iamf_mix_presentation as *const FfiIamfMixPresentation;
            if mix_ptr.is_null() || (*mix_ptr).nb_submixes == 0 || (*mix_ptr).submixes.is_null() {
                continue;
            }

            let (mix_lang, mix_annot) = extract_first_av_dict_entry((*mix_ptr).annotations);
            let nb_submixes = (*mix_ptr).nb_submixes as usize;

            for sm_idx in 0..nb_submixes {
                let submix_ptr = *(*mix_ptr).submixes.add(sm_idx);
                if submix_ptr.is_null()
                    || (*submix_ptr).nb_elements == 0
                    || (*submix_ptr).elements.is_null()
                {
                    continue;
                }
                let submix_gain = iamf_rational_db_to_linear((*submix_ptr).default_mix_gain);
                let nb_elems = (*submix_ptr).nb_elements as usize;

                let mut combined_substreams = Vec::new();
                let mut combined_mappings = Vec::new();
                let mut max_recon_ch = 1u16;
                let mut primary_layout_label = String::new();
                let mut elem_annot_opt = None;
                let mut elem_lang_opt = None;

                for el_idx in 0..nb_elems {
                    let sub_el = *(*submix_ptr).elements.add(el_idx);
                    if sub_el.is_null() {
                        continue;
                    }
                    let target_id = (*sub_el).audio_element_id as i64;
                    let elem_gain =
                        submix_gain * iamf_rational_db_to_linear((*sub_el).default_mix_gain);
                    let (el_lang, el_annot) = extract_first_av_dict_entry((*sub_el).annotations);
                    if elem_lang_opt.is_none() {
                        elem_lang_opt = el_lang;
                    }
                    if elem_annot_opt.is_none() {
                        elem_annot_opt = el_annot;
                    }

                    if let Some((_, elem_group)) =
                        audio_elements.iter().find(|(id, _)| *id == target_id)
                    {
                        used_element_ids.insert(target_id);
                        for &s_idx in &elem_group.substream_indices {
                            if !combined_substreams.contains(&s_idx) {
                                combined_substreams.push(s_idx);
                            }
                        }
                        if elem_group.reconstructed_channels >= max_recon_ch {
                            max_recon_ch = elem_group.reconstructed_channels;
                            primary_layout_label = elem_group.layout_label.clone();
                        }
                        for mut m in elem_group.channel_mappings.clone() {
                            if (elem_gain - 1.0).abs() > 1e-4 {
                                for t in &mut m.targets {
                                    t.1 *= elem_gain;
                                }
                            }
                            combined_mappings.push(m);
                        }
                    }
                }

                if !combined_substreams.is_empty() {
                    let annot = match (mix_annot.as_deref(), elem_annot_opt.as_deref()) {
                        (Some(ma), Some(ea)) if ma != ea && ea != primary_layout_label => {
                            Some(format!("{} - {}", ma, ea))
                        }
                        (Some(ma), _) if ma != primary_layout_label => Some(ma.to_string()),
                        (_, Some(ea)) if ea != primary_layout_label => Some(ea.to_string()),
                        _ => None,
                    };
                    result.push(IamfTrackGroup {
                        substream_indices: combined_substreams,
                        reconstructed_channels: max_recon_ch,
                        layout_label: primary_layout_label,
                        annotation: annot,
                        language: mix_lang.clone().or(elem_lang_opt),
                        channel_mappings: combined_mappings,
                    });
                }
            }
        }

        // 3. Include any unreferenced IAMF Audio Elements directly
        for (id, elem_group) in audio_elements {
            if !used_element_ids.contains(&id) {
                result.push(elem_group);
            }
        }
    }

    result
}

/// Estimates or scans the duration in seconds of a standalone `.iamf` OBU stream when
/// container header duration is unavailable (`AV_NOPTS_VALUE`).
fn probe_iamf_duration_seconds(file_path: &str) -> Option<f64> {
    let file_len = std::fs::metadata(file_path).ok()?.len();
    if file_len == 0 {
        return None;
    }
    let mut probe_ctx = ffmpeg_next::format::input(&file_path).ok()?;
    let st0 = probe_ctx.stream(0)?;
    let tb = st0.time_base();
    if tb.denominator() <= 0 || tb.numerator() <= 0 {
        return None;
    }
    let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;

    let mut max_end_pts: i64 = 0;
    let mut first_pos: Option<i64> = None;
    let mut last_pos: i64 = 0;
    let mut packets_seen = 0usize;
    let scan_all = file_len <= 32 * 1024 * 1024;

    for (stream, pkt) in probe_ctx.packets() {
        let pos = pkt.position() as i64;
        if pos > 0 {
            if first_pos.is_none() {
                first_pos = Some(pos);
            }
            if pos > last_pos {
                last_pos = pos;
            }
        }
        if stream.index() == 0 {
            let pts = pkt.pts().or_else(|| pkt.dts()).unwrap_or(0).max(0);
            let end_pts = pts + pkt.duration().max(0);
            if end_pts > max_end_pts {
                max_end_pts = end_pts;
            }
        }
        packets_seen += 1;
        if !scan_all && packets_seen >= 512 {
            break;
        }
    }

    let observed_sec = max_end_pts as f64 * tb_f64;
    if observed_sec <= 0.0 {
        return None;
    }
    if scan_all {
        Some(observed_sec)
    } else if let Some(fp) = first_pos
        && last_pos > fp
    {
        let bytes_span = (last_pos - fp) as f64;
        let total_payload = (file_len as i64 - fp).max(1) as f64;
        Some((observed_sec * total_payload / bytes_span).max(observed_sec))
    } else {
        Some(observed_sec)
    }
}

/// Detects whether a file is a standalone `.iamf` stream or an ISO-BMFF container (`.mp4`, `.m4a`, `.mov`)
/// carrying IAMF / Eclipsa Audio (`iamf` brand or `iacb` configuration box).
pub fn is_iamf_container(file_path: &str) -> bool {
    let lower = file_path.to_lowercase();
    if lower.ends_with(".iamf") {
        return true;
    }
    if !lower.ends_with(".mp4")
        && !lower.ends_with(".m4a")
        && !lower.ends_with(".mov")
        && !lower.ends_with(".m4v")
    {
        return false;
    }
    if let Ok(mut f) = std::fs::File::open(file_path) {
        use std::io::{Read, Seek, SeekFrom};
        let file_len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let head_len = (256 * 1024).min(file_len as usize);
        let mut buf = vec![0u8; head_len];
        if let Ok(n) = f.read(&mut buf) {
            buf.truncate(n);
            if buf.windows(4).any(|w| w == b"iamf" || w == b"iacb") {
                return true;
            }
        }
        if file_len > 256 * 1024 {
            let tail_len = (256 * 1024).min((file_len - 256 * 1024) as usize);
            let mut tail = vec![0u8; tail_len];
            if f.seek(SeekFrom::End(-(tail_len as i64))).is_ok()
                && let Ok(n) = f.read(&mut tail)
            {
                tail.truncate(n);
                if tail.windows(4).any(|w| w == b"iamf" || w == b"iacb") {
                    return true;
                }
            }
        }
    }
    false
}

struct ActiveFfmpegTrack {
    track_idx: usize,
    stream_index: usize,
    decoder: ffmpeg_next::decoder::Audio,
    resampler: ffmpeg_next::software::resampling::Context,
    sample_buf: Vec<f32>,
    buf_pos: usize,
    volume: f32,
    enabled: bool,
    channels: u16,
    time_base: f64,
    current_time: f64,
    seek_in_progress: bool,
    substream_bitrate_kbps: Option<u32>,
}

struct FfmpegSource {
    ictx: ffmpeg_next::format::context::Input,
    _custom_io: Option<crate::iso::CustomAvioGuard>,
    active_tracks: std::collections::HashMap<usize, ActiveFfmpegTrack>,
    primary_stream_index: usize,
    iamf_groups: Vec<Option<IamfTrackGroup>>,

    channels: u16,
    time_base: f64,
    current_time: f64,
    duration: f64,
    artist: String,
    ext_type: String,
    intrinsic_sample_rate: u32,
    video_info: Option<String>,
    channel_vus: Vec<f32>,
    video_stream_index: Option<usize>,
    video_tx: Option<crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>>,
    video_epoch: u64,
    video_params: Option<ffmpeg_next::codec::Parameters>,
    video_time_base: Option<ffmpeg_next::Rational>,
    video_rotation: u32,
    video_fps: f32,
    video_bitrate_kbps: Option<u32>,

    audio_tracks: Vec<crate::state::AudioTrackInfo>,
    audio_track_bitrates: Vec<Option<u32>>,
    selected_track_idx: usize,
    output_sample_rate: u32,
    is_eof: bool,
    target_seek_time: Option<f64>,
    is_full_mix_guide: bool,
    iso_media: Option<crate::iso::ResolvedIsoMedia>,
    video_pts_offset_ticks: Option<i64>,
}

fn safe_ffmpeg_channel_layout(
    layout: ffmpeg_next::channel_layout::ChannelLayout,
    channels: u16,
) -> ffmpeg_next::channel_layout::ChannelLayout {
    let ch = channels.max(1) as i32;
    if !layout.is_empty() && layout.channels() == ch {
        layout
    } else {
        ffmpeg_next::channel_layout::ChannelLayout::default(ch)
    }
}

impl FfmpegSource {
    fn fill_buffers(&mut self) -> bool {
        let mut progress = false;
        let target_pos_opt = self.target_seek_time;
        let has_iamf = self.iamf_groups.iter().any(|g| g.is_some());
        let is_2track = self.audio_tracks.len() == 2
            && !has_iamf
            && self.iso_media.is_none()
            && self.audio_tracks[0].channels == self.audio_tracks[1].channels
            && self.audio_tracks[0].channels <= 2
            && self.audio_tracks[0].codec == self.audio_tracks[1].codec;
        let need_t0 = is_2track && (self.is_full_mix_guide || self.current_time < 3.0);

        // 1. Drain any packets currently in the active decoders
        for track in self.active_tracks.values_mut() {
            if !track.enabled
                && track.stream_index != self.primary_stream_index
                && (!need_t0 || track.track_idx != 0)
            {
                continue;
            }
            let mut decoded = ffmpeg_next::frame::Audio::empty();
            while track.decoder.receive_frame(&mut decoded).is_ok() {
                let dec_ch = decoded.channels().max(1);
                let dec_rate = if decoded.rate() > 0 {
                    decoded.rate()
                } else {
                    self.output_sample_rate
                };
                let dec_layout = safe_ffmpeg_channel_layout(decoded.channel_layout(), dec_ch);
                if (track.channels != dec_ch
                    || track.resampler.input().format != decoded.format()
                    || track.resampler.input().rate != dec_rate
                    || track.resampler.input().channel_layout != dec_layout
                    || track.resampler.output().rate != self.output_sample_rate)
                    && let Ok(new_resamp) = ffmpeg_next::software::resampling::context::Context::get(
                        decoded.format(),
                        dec_layout,
                        dec_rate,
                        ffmpeg_next::format::sample::Sample::F32(
                            ffmpeg_next::format::sample::Type::Packed,
                        ),
                        dec_layout,
                        self.output_sample_rate,
                    )
                {
                    track.resampler = new_resamp;
                    track.channels = dec_ch;
                }
                if decoded.channel_layout() != dec_layout {
                    decoded.set_channel_layout(dec_layout);
                }
                let mut resampled = ffmpeg_next::frame::Audio::empty();
                if track.resampler.run(&decoded, &mut resampled).is_ok() {
                    let data = resampled.plane::<f32>(0);
                    let requested_len = resampled
                        .samples()
                        .saturating_mul(resampled.channels() as usize);
                    let actual_data = &data[..requested_len.min(data.len())];
                    let actual_len = actual_data.len();
                    if track.buf_pos > 4096 {
                        track.sample_buf.drain(0..track.buf_pos);
                        track.buf_pos = 0;
                    }
                    if let Some(target_pos) = target_pos_opt {
                        let actual_frames = actual_len / track.channels.max(1) as usize;
                        let frame_dur = actual_frames as f64 / self.output_sample_rate as f64;
                        if track.current_time + frame_dur <= target_pos {
                            track.current_time += frame_dur;
                            progress = true;
                        } else if track.current_time < target_pos {
                            let skip_sec = target_pos - track.current_time;
                            let skip_frames =
                                (skip_sec * self.output_sample_rate as f64).round() as usize;
                            let skip_samples =
                                (skip_frames * track.channels as usize).min(actual_len);
                            track
                                .sample_buf
                                .extend_from_slice(&actual_data[skip_samples..]);
                            track.current_time = target_pos;
                            progress = true;
                        } else {
                            track.sample_buf.extend_from_slice(actual_data);
                            progress = true;
                        }
                    } else {
                        track.sample_buf.extend_from_slice(actual_data);
                        progress = true;
                    }
                }
            }
        }

        // 2. Read packets from container if any active track still needs data
        let mut need_packets = false;
        for track in self.active_tracks.values() {
            if !track.enabled
                && track.stream_index != self.primary_stream_index
                && (!need_t0 || track.track_idx != 0)
            {
                continue;
            }
            let avail = (track.sample_buf.len().saturating_sub(track.buf_pos))
                / track.channels.max(1) as usize;
            if avail < 1024 {
                need_packets = true;
                break;
            }
        }

        if need_packets && !self.is_eof {
            let max_packets = if self.target_seek_time.is_some() {
                4096
            } else {
                512
            };
            for _ in 0..max_packets {
                let all_satisfied = self
                    .active_tracks
                    .values()
                    .filter(|t| {
                        t.enabled
                            || t.stream_index == self.primary_stream_index
                            || (need_t0 && t.track_idx == 0)
                    })
                    .all(|t| {
                        let avail = (t.sample_buf.len().saturating_sub(t.buf_pos))
                            / t.channels.max(1) as usize;
                        avail >= 1024
                    });
                if all_satisfied {
                    break;
                }

                if let Some((stream, packet)) = self.ictx.packets().next() {
                    let s_idx = stream.index();
                    if Some(s_idx) == self.video_stream_index {
                        if let Some(tx) = &self.video_tx {
                            let mut v_pkt = packet.clone();
                            if self.iso_media.is_some()
                                && let Some(tb) = self.video_time_base
                            {
                                let tb_f64 = tb.numerator() as f64 / tb.denominator().max(1) as f64;
                                if tb_f64 > 0.0
                                    && let Some(raw_ts) = v_pkt.pts().or_else(|| v_pkt.dts())
                                {
                                    let raw_sec = raw_ts as f64 * tb_f64;
                                    let need_reanchor = match self.video_pts_offset_ticks {
                                        None => true,
                                        Some(off) => {
                                            let adjusted_sec = (raw_ts + off) as f64 * tb_f64;
                                            (adjusted_sec - self.current_time).abs() > 4.0
                                        }
                                    };
                                    if need_reanchor {
                                        let desired_ticks =
                                            (self.current_time / tb_f64).round() as i64;
                                        self.video_pts_offset_ticks = Some(desired_ticks - raw_ts);
                                        let _ = raw_sec;
                                    }
                                    if let Some(off) = self.video_pts_offset_ticks {
                                        if let Some(pts) = v_pkt.pts() {
                                            v_pkt.set_pts(Some(pts + off));
                                        }
                                        if let Some(dts) = v_pkt.dts() {
                                            v_pkt.set_dts(Some(dts + off));
                                        }
                                    }
                                }
                            }
                            let _ = tx.try_send((self.video_epoch, v_pkt));
                        }
                    } else if let Some(track) = self.active_tracks.get_mut(&s_idx)
                        && (track.enabled
                            || track.stream_index == self.primary_stream_index
                            || (need_t0 && track.track_idx == 0))
                    {
                        if track.seek_in_progress {
                            if let Some(pkt_pts) = packet.pts().or_else(|| packet.dts()) {
                                track.current_time = pkt_pts as f64 * track.time_base;
                            } else {
                                track.current_time = self.current_time;
                            }
                            track.seek_in_progress = false;
                        }
                        let _ = track.decoder.send_packet(&packet);
                        let mut decoded = ffmpeg_next::frame::Audio::empty();
                        while track.decoder.receive_frame(&mut decoded).is_ok() {
                            let dec_ch = decoded.channels().max(1);
                            let dec_rate = if decoded.rate() > 0 {
                                decoded.rate()
                            } else {
                                self.output_sample_rate
                            };
                            if track.substream_bitrate_kbps.is_none() {
                                if track.decoder.bit_rate() > 0 {
                                    track.substream_bitrate_kbps =
                                        Some((track.decoder.bit_rate() / 1000).max(1) as u32);
                                } else if decoded.samples() > 0 && dec_rate > 0 && packet.size() > 0
                                {
                                    let frame_dur = decoded.samples() as f64 / dec_rate as f64;
                                    if frame_dur > 0.0005 {
                                        let kbps = ((packet.size() as f64 * 8.0)
                                            / (frame_dur * 1000.0))
                                            .round()
                                            as u32;
                                        if kbps > 0 {
                                            track.substream_bitrate_kbps = Some(kbps);
                                        }
                                    }
                                }
                            }
                            if !has_iamf
                                && let Some(slot) =
                                    self.audio_track_bitrates.get_mut(track.track_idx)
                                && slot.is_none()
                            {
                                *slot = track.substream_bitrate_kbps;
                            }
                            let dec_layout =
                                safe_ffmpeg_channel_layout(decoded.channel_layout(), dec_ch);
                            if (track.channels != dec_ch
                                || track.resampler.input().format != decoded.format()
                                || track.resampler.input().rate != dec_rate
                                || track.resampler.input().channel_layout != dec_layout
                                || track.resampler.output().rate != self.output_sample_rate)
                                && let Ok(new_resamp) =
                                    ffmpeg_next::software::resampling::context::Context::get(
                                        decoded.format(),
                                        dec_layout,
                                        dec_rate,
                                        ffmpeg_next::format::sample::Sample::F32(
                                            ffmpeg_next::format::sample::Type::Packed,
                                        ),
                                        dec_layout,
                                        self.output_sample_rate,
                                    )
                            {
                                track.resampler = new_resamp;
                                track.channels = dec_ch;
                            }
                            if decoded.channel_layout() != dec_layout {
                                decoded.set_channel_layout(dec_layout);
                            }
                            let mut resampled = ffmpeg_next::frame::Audio::empty();
                            if track.resampler.run(&decoded, &mut resampled).is_ok() {
                                let data = resampled.plane::<f32>(0);
                                let requested_len = resampled
                                    .samples()
                                    .saturating_mul(resampled.channels() as usize);
                                let actual_data = &data[..requested_len.min(data.len())];
                                let actual_len = actual_data.len();
                                if track.buf_pos > 4096 {
                                    track.sample_buf.drain(0..track.buf_pos);
                                    track.buf_pos = 0;
                                }
                                if let Some(target_pos) = self.target_seek_time {
                                    let actual_frames = actual_len / track.channels.max(1) as usize;
                                    let frame_dur =
                                        actual_frames as f64 / self.output_sample_rate as f64;
                                    if track.current_time + frame_dur <= target_pos {
                                        track.current_time += frame_dur;
                                        progress = true;
                                    } else if track.current_time < target_pos {
                                        let skip_sec = target_pos - track.current_time;
                                        let skip_frames =
                                            (skip_sec * self.output_sample_rate as f64).round()
                                                as usize;
                                        let skip_samples =
                                            (skip_frames * track.channels as usize).min(actual_len);
                                        track
                                            .sample_buf
                                            .extend_from_slice(&actual_data[skip_samples..]);
                                        track.current_time = target_pos;
                                        progress = true;
                                    } else {
                                        track.sample_buf.extend_from_slice(actual_data);
                                        progress = true;
                                    }
                                } else {
                                    track.sample_buf.extend_from_slice(actual_data);
                                    progress = true;
                                }
                            }
                        }
                    }
                } else {
                    self.is_eof = true;
                    break;
                }
            }
        }

        if has_iamf {
            for (t_idx, grp_opt) in self.iamf_groups.iter().enumerate() {
                if let Some(grp) = grp_opt
                    && let Some(slot) = self.audio_track_bitrates.get_mut(t_idx)
                    && slot.is_none()
                {
                    let mut sum_kbps = 0u32;
                    let mut all_known = true;
                    for &s_idx in &grp.substream_indices {
                        if let Some(st) = self.active_tracks.get(&s_idx)
                            && let Some(br) = st.substream_bitrate_kbps
                        {
                            sum_kbps += br;
                        } else {
                            all_known = false;
                            break;
                        }
                    }
                    if all_known && sum_kbps > 0 {
                        *slot = Some(sum_kbps);
                    }
                }
            }
        } else if let Some(primary) = self.active_tracks.get(&self.primary_stream_index)
            && self.channels != primary.channels
        {
            self.channels = primary.channels;
            if self.channel_vus.len() < self.channels as usize {
                self.channel_vus.resize(self.channels as usize, 0.0);
            }
        }

        if let Some(target_pos) = self.target_seek_time {
            let all_reached = self
                .active_tracks
                .values()
                .filter(|t| t.enabled || t.stream_index == self.primary_stream_index)
                .all(|t| !t.seek_in_progress && t.current_time >= target_pos);
            if all_reached {
                self.target_seek_time = None;
            }
        }

        progress
    }
}

impl AudioSource for FfmpegSource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        if sample_rate > 0 && sample_rate != self.output_sample_rate {
            self.output_sample_rate = sample_rate;
            for track in self.active_tracks.values_mut() {
                let layout =
                    safe_ffmpeg_channel_layout(track.decoder.channel_layout(), track.channels);
                let fmt = if track.decoder.format() == ffmpeg_next::format::sample::Sample::None {
                    track.resampler.input().format
                } else {
                    track.decoder.format()
                };
                let in_rate = if track.decoder.rate() > 0 {
                    track.decoder.rate()
                } else {
                    track.resampler.input().rate
                };
                if let Ok(resamp) = ffmpeg_next::software::resampling::context::Context::get(
                    fmt,
                    layout,
                    in_rate,
                    ffmpeg_next::format::sample::Sample::F32(
                        ffmpeg_next::format::sample::Type::Packed,
                    ),
                    layout,
                    self.output_sample_rate,
                ) {
                    track.resampler = resamp;
                }
            }
        }
        let mut frames_written = 0;
        let frames_needed = output.len() / hardware_channels;
        self.channel_vus.fill(0.0);

        let has_iamf = self.iamf_groups.iter().any(|g| g.is_some());
        let is_2track = self.audio_tracks.len() == 2
            && !has_iamf
            && self.iso_media.is_none()
            && self.audio_tracks[0].channels == self.audio_tracks[1].channels
            && self.audio_tracks[0].channels <= 2
            && self.audio_tracks[0].codec == self.audio_tracks[1].codec;

        // Dynamic energy-cancellation check for 2-track stereo audio (instrumental + full mix vocal guide)
        if !self.is_full_mix_guide && is_2track {
            let t0_opt = self.active_tracks.values().find(|t| t.track_idx == 0);
            let t1_opt = self.active_tracks.values().find(|t| t.track_idx == 1);
            if let (Some(t0), Some(t1)) = (t0_opt, t1_opt) {
                let ch0 = t0.channels as usize;
                let ch1 = t1.channels as usize;
                if ch0 == ch1 && ch0 <= 2 {
                    let avail0 = (t0.sample_buf.len().saturating_sub(t0.buf_pos)) / ch0.max(1);
                    let avail1 = (t1.sample_buf.len().saturating_sub(t1.buf_pos)) / ch1.max(1);
                    let check_frames = avail0.min(avail1).min(1024);
                    if check_frames >= 512 {
                        let mut e0 = 0.0f32;
                        let mut e1 = 0.0f32;
                        let mut ediff = 0.0f32;
                        for i in 0..check_frames {
                            let s0 = t0
                                .sample_buf
                                .get(t0.buf_pos + i * ch0)
                                .copied()
                                .unwrap_or(0.0);
                            let s1 = t1
                                .sample_buf
                                .get(t1.buf_pos + i * ch1)
                                .copied()
                                .unwrap_or(0.0);
                            e0 += s0 * s0;
                            e1 += s1 * s1;
                            ediff += (s1 - s0) * (s1 - s0);
                        }
                        let min_energy = 0.02 * (check_frames as f32);
                        if e0 > min_energy
                            && e1 > min_energy
                            && ediff > 0.005 * e1
                            && ediff < 0.30 * e1
                        {
                            self.is_full_mix_guide = true;
                            eprintln!(
                                "[RustTracker Audio] Detected Full Mix vocal guide stem in FFmpegSource - enabling phase cancellation"
                            );
                        }
                    }
                }
            }
        }

        let need_t0 = is_2track && (self.is_full_mix_guide || self.current_time < 3.0);

        while frames_written < frames_needed {
            let mut min_available = usize::MAX;
            for track in self.active_tracks.values().filter(|t| {
                t.enabled
                    || t.stream_index == self.primary_stream_index
                    || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.sample_buf.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                min_available = min_available.min(avail);
            }

            if min_available == 0 || min_available == usize::MAX {
                if !self.fill_buffers() {
                    if let Some(primary) = self.active_tracks.get(&self.primary_stream_index) {
                        let avail = (primary.sample_buf.len().saturating_sub(primary.buf_pos))
                            / primary.channels.max(1) as usize;
                        if avail == 0 {
                            if self.is_eof {
                                break;
                            }
                            // Try filling additional batches (especially if skipping video packets during seek)
                            let extra_batches = if self.target_seek_time.is_some() {
                                4
                            } else {
                                1
                            };
                            let mut filled = false;
                            for _ in 0..extra_batches {
                                if self.fill_buffers() || self.is_eof {
                                    filled = !self.is_eof;
                                    break;
                                }
                            }
                            if !filled {
                                break;
                            }
                            continue;
                        }
                        min_available = avail;
                    } else {
                        break;
                    }
                } else {
                    continue;
                }
            }

            let frames_to_copy = min_available.min(frames_needed - frames_written);
            if frames_to_copy == 0 {
                break;
            }

            let headroom = calculate_mix_headroom(
                self.active_tracks
                    .values()
                    .filter(|t| t.enabled)
                    .map(|t| t.volume),
            );

            let is_full_mix = self.is_full_mix_guide && is_2track;
            let (t0_info, t1_info) = if is_full_mix {
                let mut t0 = None;
                let mut t1 = None;
                for t in self.active_tracks.values() {
                    if t.track_idx == 0 {
                        t0 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    } else if t.track_idx == 1 {
                        t1 = Some((t.buf_pos, t.channels, t.volume, t.enabled));
                    }
                }
                (t0, t1)
            } else {
                (None, None)
            };

            if is_full_mix && let (Some(t0), Some(t1)) = (t0_info, t1_info) {
                let v0 = if t0.3 { t0.2 } else { 0.0 };
                let v1 = if t1.3 { t1.2 } else { 0.0 };
                let vocal_headroom = 1.0 / ((v0 - v1).abs() + v1.abs()).max(1.0);

                let t0_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 0)
                    .unwrap()
                    .sample_buf;
                let t1_samples = &self
                    .active_tracks
                    .values()
                    .find(|t| t.track_idx == 1)
                    .unwrap()
                    .sample_buf;

                for f in 0..frames_to_copy {
                    let out_base = (frames_written + f) * hardware_channels;
                    let base_idx_0 = t0.0 + f * t0.1 as usize;
                    let base_idx_1 = t1.0 + f * t1.1 as usize;
                    let ch_slice_0 = if base_idx_0 < t0_samples.len() {
                        &t0_samples[base_idx_0..]
                    } else {
                        &[]
                    };
                    let ch_slice_1 = if base_idx_1 < t1_samples.len() {
                        &t1_samples[base_idx_1..]
                    } else {
                        &[]
                    };

                    for c in 0..hardware_channels {
                        let s0 = get_downmixed_sample(t0.1, ch_slice_0, hardware_channels, c);
                        let s1 = get_downmixed_sample(t1.1, ch_slice_1, hardware_channels, c);
                        // Extracted Vocals V = s1 - s0
                        // Desired output: v0 * I + v1 * V = (v0 - v1) * s0 + v1 * s1
                        let sum_val = ((v0 - v1) * s0 + v1 * s1) * vocal_headroom;
                        let final_val = sum_val.clamp(-1.0, 1.0);
                        output[out_base + c] = final_val;

                        if c < self.channel_vus.len() {
                            self.channel_vus[c] = self.channel_vus[c].max(final_val.abs());
                        }
                    }
                }
            } else if let Some(Some(iamf_group)) =
                self.iamf_groups.get(self.selected_track_idx).cloned()
            {
                let recon_ch = (iamf_group.reconstructed_channels as usize).clamp(1, 12);
                let track_vol = self
                    .active_tracks
                    .get(&self.primary_stream_index)
                    .map(|t| if t.enabled { t.volume } else { 1.0 })
                    .unwrap_or(1.0);

                for f in 0..frames_to_copy {
                    let out_idx = (frames_written + f) * hardware_channels;
                    let mut recon = [0.0f32; 12];

                    for mapping in &iamf_group.channel_mappings {
                        if let Some(sub_track) = self.active_tracks.get(&mapping.stream_index) {
                            let in_idx = sub_track.buf_pos + f * sub_track.channels as usize;
                            let sample = sub_track
                                .sample_buf
                                .get(in_idx + mapping.substream_channel)
                                .copied()
                                .unwrap_or(0.0);
                            if sample != 0.0 {
                                for &(dst_ch, weight) in &mapping.targets {
                                    if dst_ch < 12 {
                                        recon[dst_ch] += sample * weight;
                                    }
                                }
                            }
                        }
                    }

                    for (ch, recon_c) in recon.iter_mut().enumerate().take(recon_ch) {
                        *recon_c = (*recon_c * track_vol).clamp(-1.0, 1.0);
                        if ch < self.channel_vus.len() {
                            self.channel_vus[ch] = self.channel_vus[ch].max(recon_c.abs());
                        }
                    }

                    for c in 0..hardware_channels {
                        let s_val = get_downmixed_sample(
                            recon_ch as u16,
                            &recon[..recon_ch],
                            hardware_channels,
                            c,
                        );
                        output[out_idx + c] = s_val.clamp(-1.0, 1.0);
                    }
                }
            } else {
                for f in 0..frames_to_copy {
                    let out_idx = (frames_written + f) * hardware_channels;

                    // Update true post-mix channel VUs across all source channels with headroom applied
                    for ch in 0..self.channel_vus.len() {
                        let mut ch_sum = 0.0f32;
                        for track in self.active_tracks.values() {
                            if track.enabled && track.volume > 0.0 && ch < track.channels as usize {
                                let in_idx = track.buf_pos + f * track.channels as usize;
                                let s = track.sample_buf.get(in_idx + ch).copied().unwrap_or(0.0);
                                ch_sum += s * track.volume;
                            }
                        }
                        self.channel_vus[ch] =
                            self.channel_vus[ch].max((ch_sum * headroom).abs().min(1.0));
                    }

                    for c in 0..hardware_channels {
                        let mut sum_val = 0.0f32;
                        for track in self.active_tracks.values() {
                            if track.enabled && track.volume > 0.0 {
                                let in_idx = track.buf_pos + f * track.channels as usize;
                                let ch_slice = if in_idx < track.sample_buf.len() {
                                    &track.sample_buf[in_idx..]
                                } else {
                                    &[]
                                };
                                let s_val = get_downmixed_sample(
                                    track.channels,
                                    ch_slice,
                                    hardware_channels,
                                    c,
                                );
                                sum_val += s_val * track.volume;
                            }
                        }
                        let out_val = (sum_val * headroom).clamp(-1.0, 1.0);
                        output[out_idx + c] = out_val;
                    }
                }
            }

            for track in self.active_tracks.values_mut().filter(|t| {
                t.enabled
                    || t.stream_index == self.primary_stream_index
                    || (need_t0 && t.track_idx == 0)
            }) {
                let avail = (track.sample_buf.len().saturating_sub(track.buf_pos))
                    / track.channels.max(1) as usize;
                let consume = frames_to_copy.min(avail);
                track.buf_pos += consume * track.channels as usize;
                track.current_time += consume as f64 / self.output_sample_rate as f64;
            }
            if let Some(primary) = self.active_tracks.get(&self.primary_stream_index) {
                self.current_time = primary.current_time;
            }
            frames_written += frames_to_copy;
        }

        frames_written
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }

    fn set_position_seconds(&mut self, pos: f64) {
        self.is_eof = false;
        if let Some(ref iso_media) = self.iso_media {
            let byte_pos = iso_media.time_to_logical_byte(pos, self.duration) as i64;
            unsafe {
                ffmpeg_next::ffi::av_seek_frame(
                    self.ictx.as_mut_ptr(),
                    -1,
                    byte_pos,
                    ffmpeg_next::ffi::AVSEEK_FLAG_BYTE,
                );
            }
            self.target_seek_time = None;
            self.video_pts_offset_ticks = None;
            for track in self.active_tracks.values_mut() {
                track.decoder.flush();
                track.sample_buf.clear();
                track.buf_pos = 0;
                track.current_time = pos;
                track.seek_in_progress = false;
            }
            self.current_time = pos;
            self.video_epoch += 1;
            return;
        }
        let (stream_idx, pts) =
            if let (Some(v_idx), Some(tb)) = (self.video_stream_index, self.video_time_base) {
                let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;
                (v_idx as i32, (pos / tb_f64) as i64)
            } else {
                (
                    -1,
                    (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64).max(0.0) as i64,
                )
            };

        let ret = unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                stream_idx,
                pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
            )
        };
        if ret < 0 && stream_idx != -1 {
            let fallback_pts = (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64).max(0.0) as i64;
            unsafe {
                ffmpeg_next::ffi::av_seek_frame(
                    self.ictx.as_mut_ptr(),
                    -1,
                    fallback_pts,
                    ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                );
            }
        }
        self.target_seek_time = Some(pos);
        for track in self.active_tracks.values_mut() {
            track.decoder.flush();
            track.sample_buf.clear();
            track.buf_pos = 0;
            track.current_time = 0.0;
            track.seek_in_progress = true;
        }
        self.current_time = pos;
        self.video_epoch += 1;
    }

    fn get_num_channels(&mut self) -> i32 {
        self.channels as i32
    }
    fn get_current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        self.channel_vus
            .get(channel as usize)
            .cloned()
            .unwrap_or(0.0)
    }
    fn get_artist(&mut self) -> String {
        self.artist.clone()
    }
    fn get_type(&mut self) -> String {
        self.ext_type.clone()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        Some(self.intrinsic_sample_rate)
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn get_tracker_channels(&mut self) -> Option<i32> {
        None
    }
    fn pre_format_tracker_data(&mut self) -> Vec<Vec<String>> {
        Vec::new()
    }
    fn get_current_row_string(&mut self) -> String {
        String::new()
    }
    fn get_video_info(&mut self) -> Option<String> {
        self.video_info.clone()
    }

    fn get_bitrate(&mut self) -> Option<u32> {
        if let Some(icy_br) = self.ictx.metadata().get("icy-br")
            && let Ok(br) = icy_br.parse::<u32>()
        {
            return Some(br);
        }

        if let Some(Some(track_br)) = self.audio_track_bitrates.get(self.selected_track_idx)
            && *track_br > 0
        {
            return Some(*track_br);
        }

        let br = self.ictx.bit_rate();
        if br > 0 {
            let total_kbps = ((br as f64) / 1000.0).round() as u32;
            if let Some(v_br) = self.video_bitrate_kbps {
                let rem = total_kbps.saturating_sub(v_br);
                if rem > 0 {
                    return Some(rem);
                }
            } else if self.video_stream_index.is_none() {
                return Some(total_kbps);
            }
        }

        None
    }

    fn get_video_fps(&self) -> f32 {
        self.video_fps
    }

    fn get_video_bitrate(&self) -> Option<u32> {
        self.video_bitrate_kbps
    }

    fn attach_video_queue(&mut self, tx: crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>) {
        self.video_tx = Some(tx);
    }

    fn take_video_parameters(
        &mut self,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational, u32)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base) {
            return Some((p, tb, self.video_rotation));
        }
        None
    }

    fn get_audio_tracks(&self) -> Vec<crate::state::AudioTrackInfo> {
        self.audio_tracks.clone()
    }
    fn get_selected_audio_track(&self) -> usize {
        self.selected_track_idx
    }

    fn get_active_audio_tracks(&self) -> Vec<usize> {
        let mut list: Vec<usize> = self
            .active_tracks
            .values()
            .filter(|t| t.enabled)
            .map(|t| t.track_idx)
            .collect();
        list.sort();
        list.dedup();
        if list.is_empty() {
            vec![self.selected_track_idx]
        } else {
            list
        }
    }

    fn set_active_audio_tracks(&mut self, tracks: &[(usize, f32)]) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }

        if self.iamf_groups.iter().any(|g| g.is_some()) {
            for track in self.active_tracks.values_mut() {
                track.enabled = false;
                track.volume = 0.0;
            }
            for &(t_idx, vol) in tracks {
                if let Some(Some(grp)) = self.iamf_groups.get(t_idx) {
                    for &s_idx in &grp.substream_indices {
                        if let Some(st) = self.active_tracks.get_mut(&s_idx) {
                            st.enabled = true;
                            st.volume = vol;
                            st.track_idx = t_idx;
                        }
                    }
                } else {
                    for track in self.active_tracks.values_mut() {
                        if track.track_idx == t_idx {
                            track.enabled = true;
                            track.volume = vol;
                        }
                    }
                }
            }
        } else {
            let track_map: std::collections::HashMap<usize, f32> = tracks.iter().copied().collect();
            for track in self.active_tracks.values_mut() {
                if let Some(&vol) = track_map.get(&track.track_idx) {
                    track.enabled = true;
                    track.volume = vol;
                } else {
                    track.enabled = false;
                    track.volume = 0.0;
                }
            }
        }

        if let Some(&(first_idx, _)) = tracks.first()
            && first_idx < self.audio_tracks.len()
        {
            self.selected_track_idx = first_idx;
            self.primary_stream_index = self.audio_tracks[first_idx].id;
            self.channels = self.audio_tracks[first_idx].channels;
            self.time_base = self
                .ictx
                .stream(self.primary_stream_index)
                .map(|s| {
                    let tb = s.time_base();
                    tb.numerator() as f64 / tb.denominator() as f64
                })
                .unwrap_or(1.0 / 44100.0);
            let container = self.ext_type.split('/').nth(1).map(|s| s.to_string());
            if let Some(container) = container {
                self.ext_type = format!("{}/{}", self.audio_tracks[first_idx].codec, container);
            } else {
                self.ext_type = self.audio_tracks[first_idx].codec.clone();
            }
            self.channel_vus = vec![0.0; self.channels as usize];
        }
        Ok(())
    }

    fn select_audio_track(&mut self, track_idx: usize) -> Result<()> {
        self.set_active_audio_tracks(&[(track_idx, 1.0)])
    }

    fn has_video_stream(&self) -> bool {
        self.video_stream_index.is_some()
    }

    fn is_native_atmos(&self) -> bool {
        self.audio_tracks
            .get(self.selected_track_idx)
            .is_some_and(|t| t.is_atmos)
    }
}

// ---------------------------------------------------------
// Video Only Source (No Audio)
// ---------------------------------------------------------
struct VideoOnlySource {
    ictx: ffmpeg_next::format::context::Input,
    _custom_io: Option<crate::iso::CustomAvioGuard>,
    video_stream_index: usize,
    video_tx: Option<crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>>,
    video_epoch: u64,
    video_params: Option<ffmpeg_next::codec::Parameters>,
    video_time_base: Option<ffmpeg_next::Rational>,
    video_rotation: u32,
    current_time: f64,
    last_video_pts: f64,
    duration: f64,
    video_info: Option<String>,
    video_fps: f32,
    video_bitrate_kbps: Option<u32>,
    ext_type: String,
}

impl AudioSource for VideoOnlySource {
    fn read_frames(
        &mut self,
        hardware_channels: usize,
        sample_rate: u32,
        output: &mut [f32],
    ) -> usize {
        let frames_to_write = output.len() / hardware_channels;
        for v in output.iter_mut() {
            *v = 0.0;
        }

        if self.video_tx.is_some() {
            let tb_f64 = self
                .video_time_base
                .map(|tb| tb.numerator() as f64 / tb.denominator().max(1) as f64)
                .unwrap_or(1.0 / 30.0);
            let target_pts =
                self.current_time + (frames_to_write as f64 / sample_rate as f64) + 0.25;
            let mut packets_read = 0;
            while packets_read < 16 && self.last_video_pts <= target_pts {
                if let Some((stream, packet)) = self.ictx.packets().next() {
                    if stream.index() == self.video_stream_index {
                        if let Some(ts) = packet.dts().or_else(|| packet.pts()) {
                            let pkt_sec = ts as f64 * tb_f64;
                            if pkt_sec > self.last_video_pts {
                                self.last_video_pts = pkt_sec;
                            }
                        } else {
                            self.last_video_pts += 1.0 / 30.0;
                        }
                        if let Some(tx) = &self.video_tx {
                            let _ = tx.try_send((self.video_epoch, packet.clone()));
                        }
                        packets_read += 1;
                    }
                } else {
                    break;
                }
            }
        }

        self.current_time += frames_to_write as f64 / sample_rate as f64;
        frames_to_write
    }

    fn get_duration_seconds(&mut self) -> f64 {
        self.duration
    }
    fn get_position_seconds(&mut self) -> f64 {
        self.current_time
    }

    fn set_position_seconds(&mut self, pos: f64) {
        let (stream_idx, pts) = if let Some(tb) = self.video_time_base {
            let tb_f64 = tb.numerator() as f64 / tb.denominator() as f64;
            (self.video_stream_index as i32, (pos / tb_f64) as i64)
        } else {
            (-1, (pos * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64)
        };

        unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                self.ictx.as_mut_ptr(),
                stream_idx,
                pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
            );
        }
        self.current_time = pos;
        self.last_video_pts = pos;
        self.video_epoch += 1;
    }

    fn get_num_channels(&mut self) -> i32 {
        2
    }
    fn get_current_channel_vu_mono(&mut self, _channel: i32) -> f32 {
        0.0
    }
    fn get_artist(&mut self) -> String {
        "Video Only".to_string()
    }
    fn get_type(&mut self) -> String {
        self.ext_type.clone()
    }
    fn get_tempo(&mut self) -> i32 {
        0
    }
    fn get_speed(&mut self) -> i32 {
        0
    }
    fn get_intrinsic_sample_rate(&mut self) -> Option<u32> {
        Some(44100)
    }
    fn get_num_samples(&mut self) -> i32 {
        0
    }
    fn get_num_instruments(&mut self) -> i32 {
        0
    }
    fn get_num_patterns(&mut self) -> i32 {
        0
    }
    fn get_current_order(&mut self) -> i32 {
        0
    }
    fn get_current_row(&mut self) -> i32 {
        0
    }
    fn get_video_info(&mut self) -> Option<String> {
        self.video_info.clone()
    }
    fn get_video_fps(&self) -> f32 {
        self.video_fps
    }
    fn get_video_bitrate(&self) -> Option<u32> {
        self.video_bitrate_kbps
    }

    fn attach_video_queue(&mut self, tx: crossbeam_channel::Sender<(u64, ffmpeg_next::Packet)>) {
        self.video_tx = Some(tx);
    }

    fn take_video_parameters(
        &mut self,
    ) -> Option<(ffmpeg_next::codec::Parameters, ffmpeg_next::Rational, u32)> {
        if let (Some(p), Some(tb)) = (self.video_params.take(), self.video_time_base) {
            Some((p, tb, self.video_rotation))
        } else {
            None
        }
    }

    fn has_video_stream(&self) -> bool {
        true
    }
}

fn try_ffmpeg(file_path: &str, is_network: bool) -> Result<Box<dyn AudioSource>> {
    let _ = ffmpeg_next::init();
    let resolved_iso = if !is_network {
        crate::iso::resolve_iso_or_vob_media(std::path::Path::new(file_path))
            .ok()
            .flatten()
    } else {
        None
    };

    let (ictx, custom_io) = if let Some(ref iso_info) = resolved_iso {
        let (input, guard) = crate::iso::open_ffmpeg_input_from_iso(iso_info, file_path)
            .context("Failed to open ISO/VOB virtual stream via libavformat")?;
        (input, Some(guard))
    } else {
        let mut dict = ffmpeg_next::Dictionary::new();
        dict.set("probesize", "5000000");
        dict.set("analyzeduration", "5000000");
        let input = ffmpeg_next::format::input_with_dictionary(&file_path, dict)
            .context("Failed to open file via libavformat")?;
        (input, None)
    };

    let mut duration = ictx.duration() as f64 / ffmpeg_next::ffi::AV_TIME_BASE as f64;
    if is_network || duration < 0.0 {
        duration = 0.0;
    }
    if let Some(ref iso_info) = resolved_iso
        && let Some(ifo_dur) = iso_info.ifo_duration_sec
        && ifo_dur > 0.0
    {
        duration = ifo_dur;
    }

    if duration <= 0.0 {
        let mut max_stream_duration = 0.0;
        for stream in ictx.streams() {
            let tb = stream.time_base();
            let sd = stream.duration() as f64 * (tb.numerator() as f64 / tb.denominator() as f64);
            if sd > max_stream_duration {
                max_stream_duration = sd;
            }
        }
        if max_stream_duration > 0.0 {
            duration = max_stream_duration;
        }
    }

    let ext = if let Some(ref iso_info) = resolved_iso {
        match &iso_info.media_kind {
            crate::iso::IsoMediaKind::DvdVideo => "DVD-ISO".to_string(),
            crate::iso::IsoMediaKind::DvdAudio => "DVDA-ISO".to_string(),
            crate::iso::IsoMediaKind::DataFile { ext, .. } => format!("ISO/{}", ext.to_uppercase()),
        }
    } else {
        std::path::Path::new(file_path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("FFMPEG")
            .to_uppercase()
    };

    let detected_iamf_groups = probe_iamf_stream_groups(&ictx);
    if duration <= 0.0
        && !is_network
        && (!detected_iamf_groups.is_empty() || ext == "IAMF")
        && let Some(probed_dur) = probe_iamf_duration_seconds(file_path)
    {
        duration = probed_dur;
    }

    let mut video_info = None;
    let mut video_stream_index = None;
    let mut video_params = None;
    let mut video_tb = None;
    let mut video_rotation = 0u32;
    let mut video_fps = 0.0f32;
    let mut video_bitrate_kbps = None;
    if let Some(vs) = probe_video_stream(&ictx) {
        video_stream_index = Some(vs.stream_index);
        video_params = Some(vs.params);
        video_tb = Some(vs.time_base);
        video_rotation = vs.rotation;
        video_info = Some(vs.info);
        video_fps = vs.fps;
        video_bitrate_kbps = vs.bitrate_kbps;
    }

    let mut audio_tracks = Vec::new();
    let mut audio_track_bitrates: Vec<Option<u32>> = Vec::new();
    let mut iamf_groups: Vec<Option<IamfTrackGroup>> = Vec::new();
    let mut selected_track_idx = 0;

    let mp4_track_names = if file_path.to_lowercase().ends_with(".mp4")
        || file_path.to_lowercase().ends_with(".m4a")
        || file_path.to_lowercase().ends_with(".mov")
    {
        extract_mp4_track_names(file_path)
    } else {
        Vec::new()
    };

    let mut iamf_substream_set = std::collections::HashSet::new();
    for grp in &detected_iamf_groups {
        for &s_idx in &grp.substream_indices {
            iamf_substream_set.insert(s_idx);
        }
    }

    for grp in detected_iamf_groups {
        let primary_s_idx = grp.substream_indices[0];
        let mut sub_codec_name = "Opus".to_string();
        let mut rate = 48000u32;
        let mut lang = None;
        let mut total_br_kbps = 0u32;
        let mut any_br = false;

        for (i, &s_idx) in grp.substream_indices.iter().enumerate() {
            if let Some(stream) = ictx.stream(s_idx) {
                let p = stream.parameters();
                if i == 0 {
                    let codec_id = p.id();
                    sub_codec_name = match codec_id {
                        ffmpeg_next::codec::Id::OPUS => "Opus".to_string(),
                        ffmpeg_next::codec::Id::FLAC => "FLAC".to_string(),
                        ffmpeg_next::codec::Id::AAC => "AAC".to_string(),
                        ffmpeg_next::codec::Id::PCM_S16LE
                        | ffmpeg_next::codec::Id::PCM_S24LE
                        | ffmpeg_next::codec::Id::PCM_S32LE
                        | ffmpeg_next::codec::Id::PCM_F32LE => "PCM".to_string(),
                        _ => format!("{:?}", codec_id).to_uppercase(),
                    };
                    let meta = stream.metadata();
                    lang = meta
                        .get("language")
                        .or_else(|| meta.get("LANG"))
                        .map(|s| s.to_string())
                        .filter(|l| l.to_lowercase() != "und");
                }
                let mut sub_br = probe_stream_bitrate_kbps(&stream);
                if let Ok(ctx) = ffmpeg_next::codec::context::Context::from_parameters(p)
                    && let Ok(dec) = ctx.decoder().audio()
                {
                    if i == 0 && dec.rate() > 0 {
                        rate = dec.rate();
                    }
                    if sub_br.is_none() && dec.bit_rate() > 0 {
                        sub_br = Some((dec.bit_rate() / 1000).max(1) as u32);
                    }
                }
                if let Some(br) = sub_br {
                    total_br_kbps += br;
                    any_br = true;
                }
            }
        }

        let codec_name = format!("IAMF ({})", sub_codec_name);
        let channels = grp.reconstructed_channels;
        let ch_desc = if !grp.layout_label.is_empty() {
            grp.layout_label.clone()
        } else {
            match channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                4 => "4.0".to_string(),
                6 => "5.1".to_string(),
                8 => "7.1".to_string(),
                10 => "7.1.2".to_string(),
                12 => "7.1.4".to_string(),
                n => format!("{} ch", n),
            }
        };

        let lang = grp.language.clone().or(lang);
        let title = grp
            .annotation
            .clone()
            .or_else(|| mp4_track_names.get(audio_tracks.len()).cloned());

        let mut desc_parts = Vec::new();
        desc_parts.push(codec_name.clone());
        desc_parts.push(ch_desc);
        if rate > 0 {
            desc_parts.push(format!("{}Hz", rate));
        }

        let technical = desc_parts.join(" ");
        let display_title = match (lang.as_deref(), title.as_deref()) {
            (Some(l), Some(t)) => format!(
                "Track {}: {} [{} - {}]",
                audio_tracks.len() + 1,
                technical,
                l,
                t
            ),
            (Some(l), None) => {
                format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, l)
            }
            (None, Some(t)) => {
                format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, t)
            }
            (None, None) => format!("Track {}: {}", audio_tracks.len() + 1, technical),
        };

        audio_tracks.push(crate::state::AudioTrackInfo {
            id: primary_s_idx,
            title: display_title,
            codec: codec_name,
            channels,
            sample_rate: rate,
            language: lang,
            is_atmos: false,
        });
        audio_track_bitrates.push(if any_br { Some(total_br_kbps) } else { None });
        iamf_groups.push(Some(grp));
    }

    for stream in ictx.streams() {
        if stream.parameters().medium() == ffmpeg_next::media::Type::Audio {
            let s_idx = stream.index();
            if iamf_substream_set.contains(&s_idx) {
                continue;
            }
            let p = stream.parameters();
            let codec_id = p.id();
            let codec_name = match codec_id {
                ffmpeg_next::codec::Id::TRUEHD => "TrueHD".to_string(),
                ffmpeg_next::codec::Id::EAC3 => "E-AC3".to_string(),
                ffmpeg_next::codec::Id::AC3 => "AC3".to_string(),
                ffmpeg_next::codec::Id::DTS => "DTS".to_string(),
                ffmpeg_next::codec::Id::FLAC => "FLAC".to_string(),
                ffmpeg_next::codec::Id::AAC => "AAC".to_string(),
                ffmpeg_next::codec::Id::MP3 => "MP3".to_string(),
                ffmpeg_next::codec::Id::VORBIS => "Vorbis".to_string(),
                ffmpeg_next::codec::Id::OPUS => "Opus".to_string(),
                ffmpeg_next::codec::Id::PCM_S16LE
                | ffmpeg_next::codec::Id::PCM_S24LE
                | ffmpeg_next::codec::Id::PCM_S32LE
                | ffmpeg_next::codec::Id::PCM_F32LE => "PCM".to_string(),
                ffmpeg_next::codec::Id::PCM_DVD
                | ffmpeg_next::codec::Id::PCM_BLURAY
                | ffmpeg_next::codec::Id::PCM_S16BE
                | ffmpeg_next::codec::Id::PCM_S24BE => "LPCM".to_string(),
                ffmpeg_next::codec::Id::MP2 => "MP2".to_string(),
                _ => format!("{:?}", codec_id).to_uppercase(),
            };

            let mut track_br = probe_stream_bitrate_kbps(&stream);
            let mut dec_profile = unsafe { (*p.as_ptr()).profile };
            let (channels, rate) = if let Ok(ctx) =
                ffmpeg_next::codec::context::Context::from_parameters(p.clone())
                && let Ok(dec) = ctx.decoder().audio()
            {
                if track_br.is_none() && dec.bit_rate() > 0 {
                    track_br = Some((dec.bit_rate() / 1000).max(1) as u32);
                }
                let p_val = unsafe { (*dec.as_ptr()).profile };
                if p_val > 0 {
                    dec_profile = p_val;
                }
                (
                    dec.channels().max(1),
                    if dec.rate() > 0 { dec.rate() } else { 48000 },
                )
            } else {
                (2, 48000)
            };

            let meta = stream.metadata();
            let mut lang = meta
                .get("language")
                .or_else(|| meta.get("LANG"))
                .map(|s| s.to_string())
                .filter(|l| l.to_lowercase() != "und");
            let mut title = meta
                .get("name")
                .or_else(|| meta.get("NAME"))
                .or_else(|| meta.get("title"))
                .or_else(|| meta.get("TITLE"))
                .or_else(|| meta.get("description"))
                .map(|s| s.to_string())
                .or_else(|| mp4_track_names.get(audio_tracks.len()).cloned());

            if let Some(ref iso_info) = resolved_iso
                && let Some(ifo_aud) = iso_info.audio_streams.get(audio_tracks.len())
            {
                if lang.is_none() {
                    lang = ifo_aud.language.clone();
                }
                if title.is_none() {
                    if let Some(ref label) = ifo_aud.commentary_label {
                        title = Some(label.clone());
                    } else if let Some(bits) = ifo_aud.bit_depth
                        && bits > 16
                        && codec_name == "LPCM"
                    {
                        title = Some(format!("{}-bit", bits));
                    }
                }
            }

            // FF_PROFILE_TRUEHD_ATMOS = 30, FF_PROFILE_EAC3_DDP_ATMOS = 30
            let is_atmos = ((codec_id == ffmpeg_next::codec::Id::TRUEHD
                || codec_id == ffmpeg_next::codec::Id::EAC3)
                && dec_profile == 30)
                || title
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains("atmos"));

            let ch_desc = if is_atmos {
                "Dolby Atmos (7.1.4)".to_string()
            } else {
                match channels {
                    1 => "Mono".to_string(),
                    2 => "Stereo".to_string(),
                    6 => "5.1".to_string(),
                    8 => "7.1".to_string(),
                    10 => "7.1.2".to_string(),
                    12 => "7.1.4".to_string(),
                    n => format!("{} ch", n),
                }
            };

            let mut desc_parts = Vec::new();
            desc_parts.push(codec_name.clone());
            desc_parts.push(ch_desc);
            if rate > 0 {
                desc_parts.push(format!("{}Hz", rate));
            }

            let technical = desc_parts.join(" ");
            let display_title = match (lang.as_deref(), title.as_deref()) {
                (Some(l), Some(t)) => format!(
                    "Track {}: {} [{} - {}]",
                    audio_tracks.len() + 1,
                    technical,
                    l,
                    t
                ),
                (Some(l), None) => {
                    format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, l)
                }
                (None, Some(t)) => {
                    format!("Track {}: {} [{}]", audio_tracks.len() + 1, technical, t)
                }
                (None, None) => format!("Track {}: {}", audio_tracks.len() + 1, technical),
            };

            audio_tracks.push(crate::state::AudioTrackInfo {
                id: s_idx,
                title: display_title,
                codec: codec_name,
                channels,
                sample_rate: rate,
                language: lang,
                is_atmos,
            });
            audio_track_bitrates.push(track_br);
            iamf_groups.push(None);
        }
    }

    let container_kbps = if ictx.bit_rate() > 0 {
        Some((ictx.bit_rate() / 1000).max(1) as u32)
    } else if !is_network && duration > 0.0 {
        let byte_len = resolved_iso
            .as_ref()
            .map(|r| r.total_bytes)
            .or_else(|| std::fs::metadata(file_path).ok().map(|m| m.len()))
            .unwrap_or(0);
        if byte_len > 0 {
            let kbps = ((byte_len as f64 * 8.0) / (duration * 1000.0)).round() as u32;
            if kbps > 0 { Some(kbps) } else { None }
        } else {
            None
        }
    } else {
        None
    };

    if video_bitrate_kbps.is_none()
        && video_stream_index.is_some()
        && let Some(c_kbps) = container_kbps
    {
        let audio_sum: u32 = audio_track_bitrates.iter().flatten().copied().sum();
        if c_kbps > audio_sum {
            video_bitrate_kbps = Some(c_kbps - audio_sum);
        } else {
            video_bitrate_kbps = Some(c_kbps);
        }
    }

    if let Some(c_kbps) = container_kbps {
        if video_stream_index.is_none()
            && audio_track_bitrates.len() == 1
            && audio_track_bitrates[0].is_none()
        {
            audio_track_bitrates[0] = Some(c_kbps);
        } else if let Some(v_kbps) = video_bitrate_kbps
            && audio_track_bitrates.len() == 1
            && audio_track_bitrates[0].is_none()
            && c_kbps > v_kbps
        {
            audio_track_bitrates[0] = Some(c_kbps - v_kbps);
        }
    }

    if audio_tracks.is_empty() {
        if let Some(v_idx) = video_stream_index {
            return Ok(Box::new(VideoOnlySource {
                ictx,
                _custom_io: custom_io,
                video_stream_index: v_idx,
                video_tx: None,
                video_epoch: 0,
                video_params,
                video_time_base: video_tb,
                video_rotation,
                current_time: 0.0,
                last_video_pts: 0.0,
                duration,
                video_info,
                video_fps,
                video_bitrate_kbps,
                ext_type: ext,
            }));
        } else {
            return Err(anyhow::anyhow!(
                "No audio stream found and no video stream found"
            ));
        }
    }

    let best_audio_stream = ictx.streams().best(ffmpeg_next::media::Type::Audio);
    let stream_index = if iamf_groups.iter().any(|g| g.is_some()) {
        selected_track_idx = 0;
        audio_tracks[0].id
    } else if let Some(ref best) = best_audio_stream {
        if let Some(pos) = audio_tracks.iter().position(|t| t.id == best.index()) {
            selected_track_idx = pos;
        }
        best.index()
    } else {
        audio_tracks[0].id
    };

    let stream = ictx
        .stream(stream_index)
        .context("Audio stream not found")?;
    let channels = audio_tracks
        .get(selected_track_idx)
        .map(|t| t.channels)
        .unwrap_or(2);
    let sample_rate = audio_tracks
        .get(selected_track_idx)
        .map(|t| t.sample_rate)
        .unwrap_or(48000);
    let time_base = stream.time_base();
    let tb = time_base.numerator() as f64 / time_base.denominator() as f64;

    let mut initial_artist = ictx
        .metadata()
        .get("artist")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    if initial_artist == "Unknown"
        && let Some(ref iso_info) = resolved_iso
        && let Some(ref vol) = iso_info.volume_label
    {
        initial_artist = vol.clone();
    }

    let mut active_tracks = std::collections::HashMap::new();
    for (t_idx, track_info) in audio_tracks.iter().enumerate() {
        let stream_ids: Vec<usize> = if let Some(Some(grp)) = iamf_groups.get(t_idx) {
            grp.substream_indices.clone()
        } else {
            vec![track_info.id]
        };

        for st_id in stream_ids {
            if active_tracks.contains_key(&st_id) {
                continue;
            }
            if let Some(st) = ictx.stream(st_id)
                && let Ok(ctx) =
                    ffmpeg_next::codec::context::Context::from_parameters(st.parameters())
                && let Ok(dec) = ctx.decoder().audio()
            {
                let mut sub_br = probe_stream_bitrate_kbps(&st);
                if sub_br.is_none() && dec.bit_rate() > 0 {
                    sub_br = Some((dec.bit_rate() / 1000).max(1) as u32);
                }
                let ch = dec.channels().max(1);
                let layout = safe_ffmpeg_channel_layout(dec.channel_layout(), ch);
                let fmt = if dec.format() == ffmpeg_next::format::sample::Sample::None {
                    ffmpeg_next::format::sample::Sample::F32(
                        ffmpeg_next::format::sample::Type::Planar,
                    )
                } else {
                    dec.format()
                };
                let in_rate = if dec.rate() > 0 {
                    dec.rate()
                } else {
                    sample_rate.max(8000)
                };
                if let Ok(resamp) = ffmpeg_next::software::resampling::context::Context::get(
                    fmt,
                    layout,
                    in_rate,
                    ffmpeg_next::format::sample::Sample::F32(
                        ffmpeg_next::format::sample::Type::Packed,
                    ),
                    layout,
                    sample_rate.max(8000),
                ) {
                    let vol = if t_idx == selected_track_idx {
                        1.0
                    } else {
                        0.0
                    };
                    let enabled = t_idx == selected_track_idx;
                    let track_st_tb = st.time_base();
                    let track_tb_f64 =
                        track_st_tb.numerator() as f64 / track_st_tb.denominator() as f64;
                    active_tracks.insert(
                        st_id,
                        ActiveFfmpegTrack {
                            track_idx: t_idx,
                            stream_index: st_id,
                            decoder: dec,
                            resampler: resamp,
                            sample_buf: Vec::new(),
                            buf_pos: 0,
                            volume: vol,
                            enabled,
                            channels: ch,
                            time_base: track_tb_f64,
                            current_time: 0.0,
                            seek_in_progress: video_stream_index.is_some()
                                && resolved_iso.is_none(),
                            substream_bitrate_kbps: sub_br,
                        },
                    );
                }
            }
        }
    }

    if !active_tracks.contains_key(&stream_index) {
        return Err(anyhow::anyhow!(
            "Failed to initialize FFmpeg audio decoder for primary stream"
        ));
    }

    let is_full_mix_guide = if audio_tracks.len() == 2 && resolved_iso.is_none() {
        let t1_title = audio_tracks[1].title.to_lowercase();
        t1_title.contains("full mix") || t1_title.contains("fullmix")
    } else {
        false
    };

    Ok(Box::new(FfmpegSource {
        ictx,
        _custom_io: custom_io,
        active_tracks,
        primary_stream_index: stream_index,
        channels,
        time_base: tb,
        current_time: 0.0,
        duration,
        artist: initial_artist,
        ext_type: audio_tracks
            .get(selected_track_idx)
            .map(|t| t.codec.clone())
            .unwrap_or(ext),
        intrinsic_sample_rate: sample_rate,
        video_info,
        video_fps,
        video_bitrate_kbps,
        audio_track_bitrates,
        channel_vus: vec![0.0; channels as usize],
        video_stream_index,
        video_tx: None,
        video_epoch: 0,
        video_params,
        video_time_base: video_tb,
        video_rotation,
        audio_tracks,
        selected_track_idx,
        output_sample_rate: sample_rate,
        is_eof: false,
        target_seek_time: None,
        is_full_mix_guide,
        iamf_groups,
        iso_media: resolved_iso,
        video_pts_offset_ticks: None,
    }))
}

/// Helper to extract human-readable track names (e.g. "Instrumental (Karaoke)", "Vocals (Guide)")
/// from ISO-BMFF / MP4 containers when present in `udta.name` or `name` atoms.
pub fn extract_mp4_track_names(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(mut file) = std::fs::File::open(path) {
        use std::io::{Read, Seek, SeekFrom};
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut buffers = Vec::new();

        // 1. First 2MB (for faststart/web-optimized MP4s where moov is at the start)
        let mut head = vec![0u8; (2 * 1024 * 1024).min(file_len as usize)];
        if let Ok(n) = file.read(&mut head) {
            head.truncate(n);
            buffers.push(head);
        }

        // 2. If file > 2MB, also read last 4MB where moov is placed in standard MP4s
        if file_len > 2 * 1024 * 1024 {
            let tail_len = (4 * 1024 * 1024).min(file_len - 2 * 1024 * 1024) as usize;
            let tail_offset = file_len - tail_len as u64;
            if file.seek(SeekFrom::Start(tail_offset)).is_ok() {
                let mut tail = vec![0u8; tail_len];
                if let Ok(n) = file.read(&mut tail) {
                    tail.truncate(n);
                    buffers.push(tail);
                }
            }
        }

        let name_marker = b"name";
        for data in &buffers {
            let mut i = 0;
            while i + 8 < data.len() {
                if &data[i..i + 4] == name_marker {
                    let start = i + 4;
                    let mut end = start;
                    while end < data.len()
                        && end < start + 64
                        && data[end] >= 0x20
                        && data[end] <= 0x7E
                    {
                        end += 1;
                    }
                    if end > start + 2
                        && let Ok(name_str) = std::str::from_utf8(&data[start..end])
                    {
                        let trimmed = name_str.trim();
                        if !trimmed.is_empty() && !names.contains(&trimmed.to_string()) {
                            names.push(trimmed.to_string());
                        }
                    }
                }
                i += 1;
            }
        }
    }
    names
}

pub fn try_symphonia<R: symphonia::core::io::MediaSource + 'static>(
    file: R,
    probe_ext: &str,
    display_ext: &str,
    video_info: Option<String>,
    file_path: Option<&str>,
) -> Result<Box<dyn AudioSource>> {
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(probe_ext);

    let format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .context("Unsupported audio format")?;

    let mp4_track_names = if let Some(p) = file_path {
        if p.to_lowercase().ends_with(".mp4")
            || p.to_lowercase().ends_with(".m4a")
            || p.to_lowercase().ends_with(".mov")
        {
            extract_mp4_track_names(p)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let has_video = format
        .tracks()
        .iter()
        .any(|t| matches!(t.codec_params, Some(CodecParameters::Video(_))))
        || probe_ext == "mp4"
        || probe_ext == "mkv"
        || probe_ext == "webm"
        || probe_ext == "mov"
        || probe_ext == "avi";

    let mut audio_tracks = Vec::new();
    let mut selected_track = None;
    let mut selected_track_idx = 0;
    let mut audio_track_count = 0;

    for track in format.tracks().iter() {
        let Some(CodecParameters::Audio(ref audio_params)) = track.codec_params else {
            continue;
        };
        let is_false_mp3 = audio_params.codec == audio_codecs::CODEC_ID_MP3
            && !probe_ext.is_empty()
            && !matches!(probe_ext, "mp3" | "mp2" | "mp1" | "mpga" | "mpeg" | "wav");
        if !is_false_mp3
            && audio_params.codec != CODEC_ID_NULL_AUDIO
            && symphonia::default::get_codecs()
                .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
                .is_ok()
        {
            let codec_name = match audio_params.codec {
                audio_codecs::CODEC_ID_AAC => "AAC".to_string(),
                audio_codecs::CODEC_ID_MP3 => "MP3".to_string(),
                audio_codecs::CODEC_ID_FLAC => "FLAC".to_string(),
                audio_codecs::CODEC_ID_VORBIS => "Vorbis".to_string(),
                audio_codecs::CODEC_ID_OPUS => "Opus".to_string(),
                audio_codecs::CODEC_ID_ALAC => "ALAC".to_string(),
                _ => format!("{:?}", audio_params.codec),
            };
            let channels = audio_params
                .channels
                .as_ref()
                .map(|ch| ch.count() as u16)
                .unwrap_or(2);
            let rate = audio_params.sample_rate.unwrap_or(44100);
            let ch_desc = match channels {
                1 => "Mono".to_string(),
                2 => "Stereo".to_string(),
                6 => "5.1".to_string(),
                8 => "7.1".to_string(),
                n => format!("{} ch", n),
            };

            let track_name_opt = mp4_track_names.get(audio_track_count).cloned();
            audio_track_count += 1;

            let title = if let Some(ref name) = track_name_opt {
                format!(
                    "Track {}: {} [{} {}]",
                    audio_tracks.len() + 1,
                    name,
                    codec_name,
                    ch_desc
                )
            } else if let Some(ref lang) = track.language {
                format!(
                    "Track {}: {} [{}] {} {}Hz",
                    audio_tracks.len() + 1,
                    codec_name,
                    lang,
                    ch_desc,
                    rate
                )
            } else {
                format!(
                    "Track {}: {} {} {}Hz",
                    audio_tracks.len() + 1,
                    codec_name,
                    ch_desc,
                    rate
                )
            };

            if selected_track.is_none() {
                selected_track = Some(track.clone());
                selected_track_idx = audio_tracks.len();
            }

            audio_tracks.push(crate::state::AudioTrackInfo {
                id: track.id as usize,
                title,
                codec: codec_name,
                channels,
                sample_rate: rate,
                language: track.language.clone(),
                is_atmos: false,
            });
        }
    }

    let track = selected_track.context("No supported audio track found")?;
    let track_id = track.id;
    let track_audio_params = match track.codec_params {
        Some(CodecParameters::Audio(ref p)) => p,
        _ => anyhow::bail!("Selected track is not an audio track"),
    };
    let channels = track_audio_params
        .channels
        .as_ref()
        .map(|ch| ch.count() as u16)
        .unwrap_or(2);
    let intrinsic_sample_rate = track_audio_params.sample_rate.unwrap_or(44100);
    let time_base = track
        .time_base
        .and_then(|t| t.calc_time(Timestamp::new(1)))
        .map(|tm| tm.as_secs_f64())
        .unwrap_or(1.0 / 44100.0);
    let duration = track
        .duration
        .and_then(|d| track.time_base.and_then(|tb| tb.calc_duration(d)))
        .map(|tm| tm.as_secs_f64())
        .or_else(|| track.num_frames.map(|n| n as f64 * time_base))
        .unwrap_or(0.0);

    let mut active_tracks = std::collections::HashMap::new();
    for (t_idx, track_info) in audio_tracks.iter().enumerate() {
        if let Some(t) = format
            .tracks()
            .iter()
            .find(|t| t.id == track_info.id as u32)
            && let Some(CodecParameters::Audio(ref t_audio_params)) = t.codec_params
            && let Ok(dec) = symphonia::default::get_codecs()
                .make_audio_decoder(t_audio_params, &AudioDecoderOptions::default())
        {
            let t_channels = t_audio_params
                .channels
                .as_ref()
                .map(|ch| ch.count() as u16)
                .unwrap_or(2);
            let t_sample_rate = t_audio_params.sample_rate.unwrap_or(44100);
            let t_time_base = t
                .time_base
                .and_then(|tb| tb.calc_time(Timestamp::new(1)))
                .map(|tm| tm.as_secs_f64())
                .unwrap_or(1.0 / 44100.0);
            let vol = if t_idx == selected_track_idx {
                1.0
            } else {
                0.0
            };
            let enabled = t_idx == selected_track_idx;
            active_tracks.insert(
                t.id,
                ActiveSymphoniaTrack {
                    track_idx: t_idx,
                    track_id: t.id,
                    decoder: dec,
                    sample_buf: Vec::new(),
                    samples: Vec::new(),
                    buf_pos: 0,
                    volume: vol,
                    enabled,
                    channels: t_channels,
                    sample_rate: t_sample_rate,
                    sinc_resampler: None,
                    time_base: t_time_base,
                    current_time: 0.0,
                },
            );
        }
    }

    let bitrate = if let Some(path) = file_path
        && let Ok(meta) = std::fs::metadata(path)
        && duration > 0.0
    {
        let bytes = meta.len();
        let kbps = ((bytes as f64 * 8.0) / (duration * 1000.0)).round() as u32;
        if kbps > 0 { Some(kbps) } else { None }
    } else {
        None
    };

    let ext_type = if display_ext.eq_ignore_ascii_case("mp4")
        || display_ext.eq_ignore_ascii_case("m4a")
        || display_ext.eq_ignore_ascii_case("mov")
    {
        let first_codec = audio_tracks
            .first()
            .map(|t| t.codec.as_str())
            .unwrap_or("AAC");
        format!("{}/{}", first_codec, display_ext.to_uppercase())
    } else if display_ext.is_empty() {
        "UNKNOWN".to_string()
    } else {
        display_ext.to_uppercase()
    };

    let is_full_mix_guide = if audio_tracks.len() == 2 {
        let t1_title = audio_tracks[1].title.to_lowercase();
        t1_title.contains("full mix") || t1_title.contains("fullmix")
    } else {
        false
    };

    Ok(Box::new(SymphoniaSource {
        format,
        active_tracks,
        primary_track_id: track_id,
        time_base,
        current_time: 0.0,
        duration,
        channels,
        channel_vus: vec![0.0; channels as usize],
        artist: "Unknown".to_string(),
        ext_type,
        intrinsic_sample_rate: Some(intrinsic_sample_rate),
        output_sample_rate: intrinsic_sample_rate,
        video_info,
        audio_tracks,
        selected_track_idx,
        has_video,
        bitrate,
        is_full_mix_guide,
    }))
}

fn is_http_url(s: &str) -> bool {
    let lower = s.trim_start();
    lower
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        || lower
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

pub fn load_audio_source(file_path: &str) -> Result<Box<dyn AudioSource>> {
    unsafe {
        ffmpeg_next::ffi::av_log_set_level(ffmpeg_next::ffi::AV_LOG_FATAL);
    }
    ffmpeg_next::format::network::init();

    if is_http_url(file_path) {
        const MAX_PLAYLIST_BODY_BYTES: u64 = 2 * 1024 * 1024;
        let playlist_agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .build()
            .into();
        let stream_agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(10)))
            .build()
            .into();
        let mut actual_path = file_path.trim().to_string();
        if let Ok(mut resp) = playlist_agent.get(&actual_path).call() {
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_lowercase();
            let path_no_query = actual_path
                .split(['?', '#'])
                .next()
                .unwrap_or(&actual_path)
                .to_ascii_lowercase();
            if content_type.contains("audio/x-scpls") || path_no_query.ends_with(".pls") {
                let mut body = String::new();
                if resp
                    .body_mut()
                    .as_reader()
                    .take(MAX_PLAYLIST_BODY_BYTES)
                    .read_to_string(&mut body)
                    .is_ok()
                {
                    for line in body.lines() {
                        if let Some((key, val)) = line.trim().split_once('=')
                            && key.trim().eq_ignore_ascii_case("File1")
                            && is_http_url(val.trim())
                        {
                            actual_path = val.trim().to_string();
                            break;
                        }
                    }
                }
            } else if content_type.contains("mpegurl")
                || path_no_query.ends_with(".m3u")
                || path_no_query.ends_with(".m3u8")
            {
                let mut body = String::new();
                if resp
                    .body_mut()
                    .as_reader()
                    .take(MAX_PLAYLIST_BODY_BYTES)
                    .read_to_string(&mut body)
                    .is_ok()
                {
                    for line in body.lines() {
                        let trimmed = line.trim();
                        if !trimmed.starts_with('#') && is_http_url(trimmed) {
                            actual_path = trimmed.to_string();
                            break;
                        }
                    }
                }
            }
        }

        if let Ok(source) = try_ffmpeg(&actual_path, true) {
            return Ok(source);
        }

        if let Ok(resp) = stream_agent.get(&actual_path).call() {
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_lowercase();
            let probe_ext = if content_type.contains("mpeg") || content_type.contains("mp3") {
                "mp3"
            } else if content_type.contains("aac") {
                "aac"
            } else if content_type.contains("ogg")
                || content_type.contains("vorbis")
                || content_type.contains("opus")
            {
                "ogg"
            } else if content_type.contains("flac") {
                "flac"
            } else if content_type.contains("wav") {
                "wav"
            } else {
                std::path::Path::new(actual_path.split('?').next().unwrap_or(&actual_path))
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("mp3")
            };
            let display_ext = probe_ext.to_uppercase();

            struct SyncStreamReader<R>(std::sync::Mutex<R>);
            impl<R: std::io::Read> std::io::Read for SyncStreamReader<R> {
                fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                    self.0.get_mut().unwrap().read(buf)
                }
            }

            let reader = SyncStreamReader(std::sync::Mutex::new(resp.into_body().into_reader()));
            let ro_source = symphonia::core::io::ReadOnlySource::new(reader);
            if let Ok(source) = try_symphonia(ro_source, probe_ext, &display_ext, None, None) {
                return Ok(source);
            }
        }

        return Err(anyhow::anyhow!("Failed to open network stream"));
    }

    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let video_info = None; // Symphonia doesn't do video

    let resolved_iso_for_fallback = if matches!(ext.as_str(), "iso" | "vob" | "aob") {
        crate::iso::resolve_iso_or_vob_media(std::path::Path::new(file_path))
            .ok()
            .flatten()
    } else {
        None
    };

    // 0. If this is an ISO containing a tracker module, load via OpenMptSource so full pattern data is available.
    if let Some(ref iso_info) = resolved_iso_for_fallback
        && let crate::iso::IsoMediaKind::DataFile {
            ext: ref inner_ext, ..
        } = iso_info.media_kind
        && matches!(
            inner_ext.as_str(),
            "mod"
                | "s3m"
                | "xm"
                | "it"
                | "mptm"
                | "stm"
                | "med"
                | "okt"
                | "669"
                | "mtm"
                | "far"
                | "ult"
                | "amf"
                | "dsm"
                | "psm"
                | "umx"
                | "mo3"
                | "ptm"
                | "mdl"
                | "ams"
                | "dbm"
                | "digi"
                | "dmf"
                | "dtm"
                | "gdm"
                | "imf"
                | "j2b"
                | "m15"
                | "nst"
                | "plm"
                | "sfx"
                | "stk"
                | "wow"
        )
        && iso_info.total_bytes > 0
        && iso_info.total_bytes <= 64 * 1024 * 1024
        && let Ok(mut vstream) = crate::iso::IsoVirtualStream::new(iso_info.extents.clone())
    {
        let mut data = Vec::with_capacity(iso_info.total_bytes as usize);
        if vstream.read_to_end(&mut data).is_ok()
            && let Ok(module) = OpenMptModule::from_memory(&data)
        {
            return Ok(Box::new(OpenMptSource {
                module,
                volume: 1.0,
            }));
        }
    }

    // 1. Prioritize FFmpeg for video containers (MKV, MP4, WEBM, MOV, AVI, ISO, VOB, AOB) and raw surround bitstreams (IAMF, DTS-HD, DTS, AC3, E-AC3, TrueHD).
    if matches!(
        ext.as_str(),
        "mkv"
            | "mp4"
            | "m4v"
            | "webm"
            | "mov"
            | "avi"
            | "flv"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "vob"
            | "aob"
            | "iso"
            | "ogv"
            | "3gp"
            | "mka"
            | "m2ts"
            | "ts"
            | "mts"
            | "iamf"
            | "dtshd"
            | "dts"
            | "ac3"
            | "eac3"
            | "thd"
            | "truehd"
            | "mlp"
    ) || (ext == "m4a" && is_iamf_container(file_path))
    {
        match try_ffmpeg(file_path, false) {
            Ok(source) => return Ok(source),
            Err(err) => {
                eprintln!("FFmpeg load failed for {}: {:?}", file_path, err);
                #[cfg(target_os = "android")]
                crate::android::log_android(
                    4,
                    &format!(
                        "[RustTracker] FFmpeg primary load failed for {}: {:?}",
                        file_path, err
                    ),
                );
            }
        }
    }

    // 2. Try OpenMPT Tracker module FIRST for tracker module files
    let is_tracker_ext = matches!(
        ext.as_str(),
        "mod"
            | "s3m"
            | "xm"
            | "it"
            | "mptm"
            | "stm"
            | "med"
            | "okt"
            | "669"
            | "mtm"
            | "far"
            | "ult"
            | "amf"
            | "dsm"
            | "psm"
            | "umx"
            | "mo3"
            | "ptm"
            | "mdl"
            | "ams"
            | "dbm"
            | "digi"
            | "dmf"
            | "dtm"
            | "gdm"
            | "imf"
            | "j2b"
            | "m15"
            | "nst"
            | "plm"
            | "sfx"
            | "stk"
            | "wow"
    );
    if is_tracker_ext && let Ok(mut file) = File::open(file_path) {
        let mut data = Vec::new();
        if file.read_to_end(&mut data).is_ok()
            && let Ok(module) = OpenMptModule::from_memory(&data)
        {
            return Ok(Box::new(OpenMptSource {
                module,
                volume: 1.0,
            }));
        }
    }

    // 3. Try MIDI FIRST for MIDI files
    if ext == "mid" || ext == "midi" {
        // Look for soundfont in project dir, then fallback to executable-relative paths, system-wide paths, and macOS Resources bundle
        let mut sf_path = "assets/soundfont.sf2".to_string();
        if !std::path::Path::new(&sf_path).exists() {
            if std::path::Path::new("soundfont.sf2").exists() {
                sf_path = "soundfont.sf2".to_string();
            } else {
                let mut found = false;
                if let Ok(exe_path) = std::env::current_exe()
                    && let Some(exe_dir) = exe_path.parent()
                {
                    let test_paths = vec![
                        exe_dir.join("assets/soundfont.sf2"),
                        exe_dir.join("soundfont.sf2"),
                        exe_dir.join("../share/rusttracker/assets/soundfont.sf2"),
                        exe_dir.join("../share/rusttracker/soundfont.sf2"),
                        exe_dir.join("../Resources/soundfont.sf2"),
                        exe_dir.join("../Resources/assets/soundfont.sf2"),
                    ];
                    for tp in test_paths {
                        if tp.exists() {
                            sf_path = tp.to_string_lossy().into_owned();
                            found = true;
                            break;
                        }
                    }
                }

                // Fallback to system-wide paths on Linux/macOS/Android/BSD if not found yet
                if !found {
                    let system_paths = vec![
                        "/data/user/0/com.rusttracker.app/files/soundfont.sf2",
                        "/data/user/0/com.rusttracker.app/cache/soundfont.sf2",
                        "/data/data/com.rusttracker.app/files/soundfont.sf2",
                        "/data/data/com.rusttracker.app/cache/soundfont.sf2",
                        "/Library/Audio/Sounds/Banks/default.sf2",
                        "/opt/homebrew/share/soundfonts/default.sf2",
                        "/usr/local/share/soundfonts/default.sf2",
                        "/usr/share/sounds/sf2/FluidR3_GM.sf2",
                        "/usr/share/sounds/sf2/default.sf2",
                        "/usr/share/soundfonts/FluidR3_GM.sf2",
                        "/usr/share/soundfonts/default.sf2",
                        "/usr/share/soundfonts/FluidR3_GM.sf3",
                        "/usr/share/soundfonts/freepats-general-midi.sf2",
                    ];
                    for sp in system_paths {
                        if std::path::Path::new(sp).exists() {
                            sf_path = sp.to_string();
                            break;
                        }
                    }
                }
            }
        }
        if let Ok(source) = MidiSource::new(file_path, &sf_path, 48000) {
            return Ok(Box::new(source));
        } else {
            return Err(anyhow::anyhow!(
                "Failed to parse MIDI or missing SoundFont ({}). Please place a SoundFont in assets/soundfont.sf2 or bundled resources",
                sf_path
            ));
        }
    }

    // 4. Try Symphonia for supported audio and container formats (FLAC, MP3, WAV, AAC, M4A, OGG, OPUS, etc.)
    let mut symphonia_error = None;
    if let Some(ref iso_info) = resolved_iso_for_fallback
        && let Ok(vstream) = crate::iso::IsoVirtualStream::new(iso_info.extents.clone())
    {
        let probe_ext = match &iso_info.media_kind {
            crate::iso::IsoMediaKind::DataFile { ext: inner_ext, .. } => inner_ext.as_str(),
            _ => "mpg",
        };
        let display_ext = probe_ext.to_uppercase();
        match try_symphonia(
            vstream,
            probe_ext,
            &display_ext,
            video_info.clone(),
            Some(file_path),
        ) {
            Ok(source) => return Ok(source),
            Err(e) => {
                symphonia_error = Some(e);
            }
        }
    } else if let Ok(file) = File::open(file_path) {
        match try_symphonia(file, &ext, &ext, video_info.clone(), Some(file_path)) {
            Ok(source) => return Ok(source),
            Err(e) => {
                println!(
                    "[RustTracker] Symphonia load failed for {}: {:?}",
                    file_path, e
                );
                symphonia_error = Some(e);
            }
        }
    }

    // 4b. Fallback: Try OpenMPT if extension was missing or non-standard (e.g. Amiga prefix "mod.song" or cloud URI without extension)
    if !is_tracker_ext
        && let Ok(mut file) = File::open(file_path)
        && let Ok(meta) = file.metadata()
        && meta.len() > 0
        && meta.len() <= 64 * 1024 * 1024
    {
        let mut data = Vec::new();
        if file.read_to_end(&mut data).is_ok()
            && let Ok(module) = OpenMptModule::from_memory(&data)
        {
            return Ok(Box::new(OpenMptSource {
                module,
                volume: 1.0,
            }));
        }
    }

    // 5. Try FFmpeg native bindings as universal fallback
    let ffmpeg_result = try_ffmpeg(file_path, false);
    if let Ok(source) = ffmpeg_result {
        return Ok(source);
    } else if let Err(ref err) = ffmpeg_result {
        if let Some(ref se) = symphonia_error {
            eprintln!(
                "[RustTracker] Symphonia load failed for {}: {:?}",
                file_path, se
            );
            #[cfg(target_os = "android")]
            crate::android::log_android(
                5,
                &format!(
                    "[RustTracker] Symphonia load failed for {}: {:?}",
                    file_path, se
                ),
            );
        }
        eprintln!("FFmpeg fallback load failed for {}: {:?}", file_path, err);
        #[cfg(target_os = "android")]
        crate::android::log_android(
            5,
            &format!(
                "[RustTracker] FFmpeg fallback load failed for {}: {:?}",
                file_path, err
            ),
        );
    }

    // If all failed, return the ffmpeg error since it's the most descriptive for standard media
    ffmpeg_result
}
