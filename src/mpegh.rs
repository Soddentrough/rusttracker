//! Real-time 7.1.4 (11.1, CICP Layout 19) MPEG-H 3D Audio LC Profile encoder (`libmpeghe`)
//! and IEC 61937-13 HDMI bitstream packager for AVRs such as the Denon AVR-X3800H.
//!
//! This module encodes 12-channel `7.1.4` floating-point audio (`48,000` Hz) into ISO/IEC 23008-3
//! MHAS packets (`PACTYP_SYNC`, `PACTYP_MPEGH3DACFG`, `PACTYP_MPEGH3DAFRAME`) and encapsulates each
//! 1024-sample access unit into an IEC 61937-13 burst (`data_type = 25` / `0x19`) suitable for
//! bit-perfect HDMI transmission over WASAPI Exclusive (Windows) or PipeWire/ALSA (Linux).

use anyhow::{Result, anyhow};
use std::collections::VecDeque;

/// Standard MPEG-H 3D Audio sample rate for 7.1.4 (`CICP 19`) HDMI bitstreaming.
pub const MPEGH_SAMPLE_RATE: u32 = 48_000;
/// Core coder frame length in samples per channel at 48 kHz.
pub const MPEGH_FRAME_SAMPLES: usize = 1024;
/// Number of discrete channels in CICP Layout 19 (`7.1.4` / `11.1`).
pub const MPEGH_714_CHANNELS: usize = 12;
/// CICP Loudspeaker Layout Index 19 (`7.1.4` / `11.1`: 7.1 bed + 4 height channels).
pub const MPEGH_CICP_714: i32 = 19;
/// Default full-bandwidth bitrate (768 kbps) for 12-channel CICP 19 LC Profile Level 3.
/// Above 32 kbps/ch so IGF spectral band replication is disabled and all 11 main channels
/// encode full 20 kHz MDCT bandwidth.
pub const MPEGH_714_DEFAULT_BITRATE: i32 = 768_000;

/// IEC 61937-13 burst preamble and framing constants.
pub const IEC61937_SYNC_PA: u16 = 0xF872;
pub const IEC61937_SYNC_PB: u16 = 0x4E1F;
pub const IEC61937_DATA_TYPE_MPEGH: u8 = 25; // 0x19
const IEC_HEADER_SIZE_BYTES: usize = 8;
const IEC_PAYLOAD_HEADER_SIZE_BYTES: usize = 8;
const IEC_BURST_SPACING_SIZE_BYTES: usize = 8;
const IEC60958_FRAME_SIZE_BYTES: usize = 4;

/// MHAS Packet Types (ISO/IEC 23008-3 Table 263).
pub const MHAS_PACTYP_MPEGH3DACFG: u32 = 1;
pub const MHAS_PACTYP_MPEGH3DAFRAME: u32 = 2;
pub const MHAS_PACTYP_SYNC: u32 = 6;
pub const MHAS_SYNC_BYTE: u8 = 0xA5;

#[repr(C)]
struct RustMpeghEncCtx {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn rust_mpegh_enc_create(
        num_channels: i32,
        sample_rate: i32,
        cicp_index: i32,
        bitrate: i32,
    ) -> *mut RustMpeghEncCtx;

    fn rust_mpegh_enc_get_asc(ctx: *const RustMpeghEncCtx, out_asc_ptr: *mut *const u8) -> i32;

    fn rust_mpegh_enc_get_frame_length(ctx: *const RustMpeghEncCtx) -> i32;

    fn rust_mpegh_enc_get_input_pcm_buf(ctx: *mut RustMpeghEncCtx) -> *mut i32;

    fn rust_mpegh_enc_encode_frame(
        ctx: *mut RustMpeghEncCtx,
        out_frame_ptr: *mut *const u8,
        out_is_rap: *mut i32,
    ) -> i32;

    fn rust_mpegh_enc_destroy(ctx: *mut RustMpeghEncCtx);
}

/// Bit-level writer for ISO/IEC 23008-3 Section 14 MHAS streams.
#[derive(Default, Clone)]
pub struct MhasBitWriter {
    bytes: Vec<u8>,
    bit_pos: usize,
}

impl MhasBitWriter {
    pub fn with_capacity(cap_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(cap_bytes),
            bit_pos: 0,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.bit_pos = 0;
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[inline]
    pub fn write_bits(&mut self, val: u64, nbits: u32) {
        for i in (0..nbits).rev() {
            let bit = ((val >> i) & 1) as u8;
            let byte_idx = self.bit_pos >> 3;
            let bit_in_byte = 7 - (self.bit_pos & 7);
            if byte_idx >= self.bytes.len() {
                self.bytes.push(0);
            }
            if bit != 0 {
                self.bytes[byte_idx] |= 1u8 << bit_in_byte;
            }
            self.bit_pos += 1;
        }
    }

    /// Writes an MHAS `escapedValue(n1, n2, n3)` as specified in ISO/IEC 23008-3 Section 14.4.
    #[inline]
    pub fn write_escaped_value(&mut self, val: u64, n1: u32, n2: u32, n3: u32) {
        let max1 = (1u64 << n1) - 1;
        if val < max1 {
            self.write_bits(val, n1);
            return;
        }
        self.write_bits(max1, n1);
        let rem1 = val - max1;
        let max2 = (1u64 << n2) - 1;
        if rem1 < max2 || n3 == 0 {
            self.write_bits(rem1, n2);
            return;
        }
        self.write_bits(max2, n2);
        let rem2 = rem1 - max2;
        self.write_bits(rem2, n3);
    }

    /// Writes a single MHAS packet (`MHASPacketType`, `MHASPacketLabel`, `MHASPacketLength`, `MHASPacketPayload`).
    pub fn write_mhas_packet(&mut self, pac_type: u32, pac_label: u64, payload: &[u8]) {
        self.write_escaped_value(pac_type as u64, 3, 8, 8);
        self.write_escaped_value(pac_label, 2, 8, 32);
        self.write_escaped_value(payload.len() as u64, 11, 24, 24);
        if (self.bit_pos & 7) == 0 {
            self.bytes.extend_from_slice(payload);
            self.bit_pos += payload.len() * 8;
        } else {
            for &b in payload {
                self.write_bits(b as u64, 8);
            }
        }
    }
}

/// Encapsulates MHAS access units into IEC 61937-13 bursts (`data_type = 25` / `0x19`),
/// matching Fraunhofer IIS's `iec61937-13` reference implementation byte-for-byte.
#[derive(Clone, Debug)]
pub struct Iec61937MpeghPackager {
    /// Rate factor relative to 48 kHz: `4` (2ch @ 192 kHz = 16,384 B/frame) or `16` (8ch @ 192 kHz HBR = 65,536 B/frame).
    rate_factor: u8,
    /// Encoded `rateFactor` code in `Pc` bits 11..12 (`1` for 4x, `3` for 16x).
    rate_factor_code: u8,
    /// Total bytes per IEC 61937-13 burst (`16,384` for `rate_factor = 4`, `65,536` for `rate_factor = 16`).
    burst_repetition_period: usize,
    /// Whether to pairwise-swap bytes into 16-bit Little-Endian (`s16le`) word order for WASAPI / ALSA / PipeWire.
    swap_bytes_s16le: bool,
}

impl Iec61937MpeghPackager {
    /// Creates a new IEC 61937-13 packager.
    /// - `rate_factor`: `4` (for 2ch @ 192 kHz carrier) or `16` (for 8ch @ 192 kHz HBR carrier).
    /// - `swap_bytes_s16le`: `true` for standard `s16le` audio APIs (Windows WASAPI Exclusive, Linux ALSA/PipeWire).
    pub fn new(rate_factor: u8, swap_bytes_s16le: bool) -> Result<Self> {
        let rate_factor_code = match rate_factor {
            4 => 1u8,
            16 => 3u8,
            _ => {
                return Err(anyhow!(
                    "Unsupported IEC 61937-13 rate factor {} (must be 4 or 16)",
                    rate_factor
                ));
            }
        };
        let burst_repetition_period =
            (MPEGH_FRAME_SAMPLES * IEC60958_FRAME_SIZE_BYTES) << (rate_factor_code + 1);
        Ok(Self {
            rate_factor,
            rate_factor_code,
            burst_repetition_period,
            swap_bytes_s16le,
        })
    }

    #[inline]
    pub fn rate_factor(&self) -> u8 {
        self.rate_factor
    }

    #[inline]
    pub fn burst_size_bytes(&self) -> usize {
        self.burst_repetition_period
    }

    /// Encapsulates a single 1024-sample MHAS access unit into a fixed-size IEC 61937-13 burst.
    pub fn package_mhas_frame(&self, mhas_payload: &[u8], out_burst: &mut Vec<u8>) -> Result<()> {
        let mhas_len = mhas_payload.len();
        let headers_and_preamble = IEC_HEADER_SIZE_BYTES + 2 * IEC_PAYLOAD_HEADER_SIZE_BYTES; // 24 bytes
        let max_payload = self
            .burst_repetition_period
            .saturating_sub(headers_and_preamble + IEC_BURST_SPACING_SIZE_BYTES);
        if mhas_len > max_payload {
            return Err(anyhow!(
                "MHAS frame ({} bytes) exceeds IEC 61937-13 burst capacity ({} bytes)",
                mhas_len,
                max_payload
            ));
        }

        out_burst.clear();
        out_burst.resize(self.burst_repetition_period, 0);

        // 1. 8-byte IEC 61937-13 Burst Preamble (Pa, Pb, Pc, Pd) in Big-Endian byte order
        out_burst[0] = (IEC61937_SYNC_PA >> 8) as u8; // 0xF8
        out_burst[1] = (IEC61937_SYNC_PA & 0xFF) as u8; // 0x72
        out_burst[2] = (IEC61937_SYNC_PB >> 8) as u8; // 0x4E
        out_burst[3] = (IEC61937_SYNC_PB & 0xFF) as u8; // 0x1F
        // Pc: bits 11..12 = rate_factor_code, bits 8..10 = frameLengthCode (0 = 1024),
        //     bits 5..6 = audioMode (1 = HBR), bits 0..4 = dataType (25 = MPEG-H 3D Audio)
        out_burst[4] = self.rate_factor_code << 3;
        out_burst[5] = (1u8 << 5) | IEC61937_DATA_TYPE_MPEGH; // 0x39

        // Pd: dataBurstLength in 64-bit (8-byte) units when audioMode == 1
        let data_burst_length_bytes = mhas_len + 2 * IEC_PAYLOAD_HEADER_SIZE_BYTES;
        let pd_words = ((data_burst_length_bytes + 7) >> 3) as u16;
        out_burst[6] = (pd_words >> 8) as u8;
        out_burst[7] = (pd_words & 0xFF) as u8;

        // 2. 8-byte Payload Header #0 (dataOffset = 24, dataSize = mhas_len, pcmOffset = 0)
        let data_offset: u32 = headers_and_preamble as u32; // 24 (0x000018)
        let data_size: u32 = mhas_len as u32;
        out_burst[8] = ((data_offset >> 16) & 0xFF) as u8;
        out_burst[9] = ((data_offset >> 8) & 0xFF) as u8;
        out_burst[10] = (data_offset & 0xFF) as u8;
        out_burst[11] = ((data_size >> 16) & 0xFF) as u8;
        out_burst[12] = ((data_size >> 8) & 0xFF) as u8;
        out_burst[13] = (data_size & 0xFF) as u8;
        out_burst[14] = 0; // pcmOffset MSB
        out_burst[15] = 0; // pcmOffset LSB

        // 3. 8-byte Zero Terminator Payload Header at bytes 16..24 (already zeroed)

        // 4. MHAS Payload at bytes 24..24 + mhas_len
        out_burst[headers_and_preamble..headers_and_preamble + mhas_len]
            .copy_from_slice(mhas_payload);

        // 5. Pairwise 16-bit Little-Endian byte swap for s16le HDMI transport
        if self.swap_bytes_s16le {
            let active_len = (headers_and_preamble + mhas_len + 1) & !1;
            for chunk in out_burst[..active_len].as_chunks_mut::<2>().0 {
                chunk.swap(0, 1);
            }
        }

        Ok(())
    }
}

/// Remaps a 12-channel frame from `rusttracker`'s internal `7.1.4` speaker order:
///   `[0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR, 8:TFL, 9:TFR, 10:TRL, 11:TRR]`
/// to MPEG-H 3D Audio `CICP 19` (`11.1` / `7.1.4`) speaker order:
///   `[0:FL, 1:FR, 2:FC, 3:LFE, 4:BL(RL), 5:BR(RR), 6:SL, 7:SR, 8:TFL, 9:TFR, 10:TBL(TRL), 11:TBR(TRR)]`.
#[inline]
pub fn remap_rusttracker_714_to_cicp19(rt: &[f32; 12]) -> [f32; 12] {
    [
        rt[0],  // 0: FL
        rt[1],  // 1: FR
        rt[2],  // 2: FC
        rt[3],  // 3: LFE
        rt[6],  // 4: BL (Rear Left)
        rt[7],  // 5: BR (Rear Right)
        rt[4],  // 6: SL (Side Left)
        rt[5],  // 7: SR (Side Right)
        rt[8],  // 8: TFL (Top Front Left)
        rt[9],  // 9: TFR (Top Front Right)
        rt[10], // 10: TBL (Top Back Left)
        rt[11], // 11: TBR (Top Back Right)
    ]
}

/// Real-time 12-channel `7.1.4` (`11.1`, CICP 19) MPEG-H 3D Audio LC Profile encoder
/// with integrated MHAS packetizer and IEC 61937-13 HDMI bitstream framing.
pub struct Mpegh714Encoder {
    ctx: *mut RustMpeghEncCtx,
    raw_asc: Vec<u8>,
    frame_length: usize,
    accum_samples: usize,
    mhas_writer: MhasBitWriter,
    iec_packager: Iec61937MpeghPackager,
    burst_scratch: Vec<u8>,
    frames_encoded: u64,
}

// Safety: Each `Mpegh714Encoder` instance owns its private heap-allocated `RustMpeghEncCtx`
// and does not share mutable global state.
unsafe impl Send for Mpegh714Encoder {}

impl Mpegh714Encoder {
    /// Creates a new real-time 12-channel (`7.1.4`, CICP 19) MPEG-H 3D Audio encoder at 48,000 Hz.
    /// - `rate_factor`: `4` (for 2ch @ 192 kHz IEC 60958 carrier) or `16` (for 8ch @ 192 kHz HBR carrier).
    /// - `swap_bytes_s16le`: `true` for standard `s16le` HDMI audio streams.
    pub fn new(rate_factor: u8, swap_bytes_s16le: bool) -> Result<Self> {
        Self::with_bitrate(MPEGH_714_DEFAULT_BITRATE, rate_factor, swap_bytes_s16le)
    }

    pub fn with_bitrate(bitrate: i32, rate_factor: u8, swap_bytes_s16le: bool) -> Result<Self> {
        let iec_packager = Iec61937MpeghPackager::new(rate_factor, swap_bytes_s16le)?;
        let ctx = unsafe {
            rust_mpegh_enc_create(
                MPEGH_714_CHANNELS as i32,
                MPEGH_SAMPLE_RATE as i32,
                MPEGH_CICP_714,
                bitrate,
            )
        };
        if ctx.is_null() {
            return Err(anyhow!(
                "Failed to initialize libmpeghe 7.1.4 (CICP 19) encoder at {} Hz / {} bps",
                MPEGH_SAMPLE_RATE,
                bitrate
            ));
        }

        let mut asc_ptr: *const u8 = std::ptr::null();
        let asc_len = unsafe { rust_mpegh_enc_get_asc(ctx, &mut asc_ptr) };
        if asc_len <= 0 || asc_ptr.is_null() {
            unsafe {
                rust_mpegh_enc_destroy(ctx);
            }
            return Err(anyhow!("libmpeghe returned empty mpegh3daConfig ASC"));
        }
        let raw_asc = unsafe { std::slice::from_raw_parts(asc_ptr, asc_len as usize).to_vec() };
        let frame_length = unsafe { rust_mpegh_enc_get_frame_length(ctx) }.max(1) as usize;
        let burst_size = iec_packager.burst_size_bytes();

        Ok(Self {
            ctx,
            raw_asc,
            frame_length,
            accum_samples: 0,
            mhas_writer: MhasBitWriter::with_capacity(4096),
            iec_packager,
            burst_scratch: Vec::with_capacity(burst_size),
            frames_encoded: 0,
        })
    }

    /// Returns the raw `mpegh3daConfig` AudioSpecificConfig bytes (15 bytes for CICP 19).
    #[inline]
    pub fn raw_asc(&self) -> &[u8] {
        &self.raw_asc
    }

    /// Returns the number of audio samples per channel per MPEG-H frame (`1024`).
    #[inline]
    pub fn frame_length(&self) -> usize {
        self.frame_length
    }

    /// Returns the size in bytes of each output IEC 61937-13 burst (`16,384` or `65,536`).
    #[inline]
    pub fn iec_burst_size_bytes(&self) -> usize {
        self.iec_packager.burst_size_bytes()
    }

    /// Clears any partially accumulated input samples (e.g., on track seek).
    #[inline]
    pub fn clear_accumulator(&mut self) {
        self.accum_samples = 0;
    }

    /// Pushes a single 12-channel sample frame in `rusttracker` 7.1.4 order:
    /// `[0:FL, 1:FR, 2:FC, 3:LFE, 4:SL, 5:SR, 6:RL, 7:RR, 8:TFL, 9:TFR, 10:TRL, 11:TRR]`.
    /// When `1024` samples have been accumulated, encodes an MPEG-H 3D Audio access unit,
    /// packages it into an IEC 61937-13 burst, and appends the burst bytes to `out_iec_queue`.
    #[inline]
    pub fn push_rusttracker_714_frame(
        &mut self,
        rt_channels: &[f32; 12],
        out_iec_queue: &mut VecDeque<u8>,
    ) -> Result<bool> {
        let cicp19 = remap_rusttracker_714_to_cicp19(rt_channels);
        self.push_cicp19_frame(&cicp19, out_iec_queue)
    }

    /// Pushes a single 12-channel sample frame already in CICP 19 order:
    /// `[0:FL, 1:FR, 2:FC, 3:LFE, 4:BL, 5:BR, 6:SL, 7:SR, 8:TFL, 9:TFR, 10:TBL, 11:TBR]`.
    pub fn push_cicp19_frame(
        &mut self,
        cicp19_channels: &[f32; 12],
        out_iec_queue: &mut VecDeque<u8>,
    ) -> Result<bool> {
        let inp_ptr = unsafe { rust_mpegh_enc_get_input_pcm_buf(self.ctx) };
        if inp_ptr.is_null() {
            return Err(anyhow!("libmpeghe input PCM buffer is null"));
        }

        let base_idx = self.accum_samples * MPEGH_714_CHANNELS;
        unsafe {
            let inp_slice =
                std::slice::from_raw_parts_mut(inp_ptr, self.frame_length * MPEGH_714_CHANNELS);
            for (c, &sample) in cicp19_channels.iter().enumerate() {
                let clamped = sample.clamp(-1.0, 1.0) as f64;
                inp_slice[base_idx + c] = (clamped * 2_147_483_520.0).round() as i32;
            }
        }

        self.accum_samples += 1;
        if self.accum_samples >= self.frame_length {
            self.accum_samples = 0;
            self.encode_current_buffer()?;
            out_iec_queue.extend(&self.burst_scratch);
            return Ok(true);
        }
        Ok(false)
    }

    /// Encodes the currently filled 1024-sample input buffer into `self.mhas_writer` and `self.burst_scratch`.
    fn encode_current_buffer(&mut self) -> Result<()> {
        let mut frame_ptr: *const u8 = std::ptr::null();
        let mut is_rap: i32 = 0;
        let out_bytes =
            unsafe { rust_mpegh_enc_encode_frame(self.ctx, &mut frame_ptr, &mut is_rap) };
        if out_bytes < 0 {
            return Err(anyhow!("libmpeghe impeghe_execute failed"));
        }
        if out_bytes == 0 || frame_ptr.is_null() {
            self.burst_scratch
                .resize(self.iec_packager.burst_size_bytes(), 0);
            self.burst_scratch.fill(0);
            return Ok(());
        }

        let raw_frame = unsafe { std::slice::from_raw_parts(frame_ptr, out_bytes as usize) };

        // Build spec-compliant MHAS access unit:
        // 1. PACTYP_SYNC (type=6, label=0, len=1, 0xA5)
        // 2. PACTYP_MPEGH3DACFG (type=1, label=1) on every RAP frame (and first 4 frames for instant AVR lock)
        // 3. PACTYP_MPEGH3DAFRAME (type=2, label=1)
        self.mhas_writer.clear();
        self.mhas_writer
            .write_mhas_packet(MHAS_PACTYP_SYNC, 0, &[MHAS_SYNC_BYTE]);
        if is_rap != 0 || self.frames_encoded < 4 {
            self.mhas_writer
                .write_mhas_packet(MHAS_PACTYP_MPEGH3DACFG, 1, &self.raw_asc);
        }
        self.mhas_writer
            .write_mhas_packet(MHAS_PACTYP_MPEGH3DAFRAME, 1, raw_frame);

        self.iec_packager
            .package_mhas_frame(self.mhas_writer.as_bytes(), &mut self.burst_scratch)?;
        self.frames_encoded = self.frames_encoded.wrapping_add(1);
        Ok(())
    }

    /// Returns the most recently encoded MHAS packet bytes (useful for diagnostics and tests).
    #[inline]
    pub fn last_mhas_bytes(&self) -> &[u8] {
        self.mhas_writer.as_bytes()
    }

    /// Returns the most recently packaged IEC 61937-13 burst bytes.
    #[inline]
    pub fn last_iec_burst(&self) -> &[u8] {
        &self.burst_scratch
    }
}

impl Drop for Mpegh714Encoder {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe {
                rust_mpegh_enc_destroy(self.ctx);
            }
            self.ctx = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpegh_714_encoder_and_iec61937_13_framing() {
        let mut enc = Mpegh714Encoder::new(4, true).expect("Mpegh714Encoder::new should succeed");
        assert_eq!(enc.frame_length(), 1024);
        assert_eq!(enc.raw_asc().len(), 15);
        assert_eq!(enc.iec_burst_size_bytes(), 16384);

        let mut iec_queue = VecDeque::new();
        let freqs: [f32; 12] = [
            220.0, 277.18, 329.63, 80.0, 392.0, 440.0, 523.25, 587.33, 659.25, 783.99, 880.0,
            987.77,
        ];

        for s in 0..1024 {
            let t = s as f32 / MPEGH_SAMPLE_RATE as f32;
            let mut frame = [0.0f32; 12];
            for c in 0..12 {
                frame[c] = 0.25 * (2.0 * std::f32::consts::PI * freqs[c] * t).sin();
            }
            let produced = enc
                .push_rusttracker_714_frame(&frame, &mut iec_queue)
                .expect("push_rusttracker_714_frame should succeed");
            if s < 1023 {
                assert!(!produced);
            } else {
                assert!(produced);
            }
        }

        assert_eq!(iec_queue.len(), 16384);
        let burst = enc.last_iec_burst();
        assert_eq!(burst.len(), 16384);

        // Verify s16le IEC 61937-13 preamble words:
        // Pa = 0xF872 -> [0x72, 0xF8]
        // Pb = 0x4E1F -> [0x1F, 0x4E]
        // Pc = 0x0839 -> [0x39, 0x08] (dataType=25=0x19, audioMode=1, rateFactorCode=1)
        assert_eq!(u16::from_le_bytes([burst[0], burst[1]]), IEC61937_SYNC_PA);
        assert_eq!(u16::from_le_bytes([burst[2], burst[3]]), IEC61937_SYNC_PB);
        let pc = u16::from_le_bytes([burst[4], burst[5]]);
        assert_eq!((pc & 0x1F) as u8, IEC61937_DATA_TYPE_MPEGH);
        assert_eq!((pc >> 5) & 0x3, 1); // audioMode = 1 (HBR)
        assert_eq!((pc >> 11) & 0x3, 1); // rateFactorCode = 1 (4x)

        // Verify MHAS starts with PACTYP_SYNC (type=6, label=0, len=1, 0xA5)
        let mhas = enc.last_mhas_bytes();
        assert!(mhas.len() > 32);
        // In MHAS: type=6 (3 bits: 110), label=0 (2 bits: 00), len=1 (11 bits: 00000000001) -> 0xC001, followed by 0xA5
        assert_eq!(mhas[0], 0xC0);
        assert_eq!(mhas[1], 0x01);
        assert_eq!(mhas[2], 0xA5);
    }
}
