//! Safe, zero-allocation direct C FFI bindings to `libopenmpt` (`libopenmpt/libopenmpt.h`).
//!
//! Replaces the unmaintained `openmpt` / `openmpt-sys` Rust crates, which allocated fresh
//! `Vec<f32>` buffers on every audio chunk and unsafely passed `Vec::capacity()` to C.
//! All audio render routines here write directly into caller-provided `&mut [f32]` slices
//! using exact slice lengths with zero heap allocations.

use anyhow::{Result, bail};
use std::ffi::{CStr, c_char, c_double, c_float, c_int, c_void};
use std::ptr::{self, NonNull};

#[repr(C)]
struct OpenMptModuleRaw {
    _private: [u8; 0],
}

#[repr(C)]
struct OpenMptModuleInitialCtl {
    ctl: *const c_char,
    value: *const c_char,
}

type OpenMptLogFunc = unsafe extern "C" fn(message: *const c_char, user: *mut c_void);
type OpenMptErrorFunc = unsafe extern "C" fn(error: c_int, user: *mut c_void) -> c_int;

unsafe extern "C" {
    fn openmpt_log_func_silent(message: *const c_char, user: *mut c_void);
    fn openmpt_error_func_ignore(error: c_int, user: *mut c_void) -> c_int;
    fn openmpt_free_string(str_ptr: *const c_char);

    fn openmpt_module_create_from_memory2(
        filedata: *const c_void,
        filesize: usize,
        logfunc: Option<OpenMptLogFunc>,
        loguser: *mut c_void,
        errfunc: Option<OpenMptErrorFunc>,
        erruser: *mut c_void,
        error: *mut c_int,
        error_message: *mut *const c_char,
        ctls: *const OpenMptModuleInitialCtl,
    ) -> *mut OpenMptModuleRaw;

    fn openmpt_module_destroy(mod_ptr: *mut OpenMptModuleRaw);

    fn openmpt_module_set_repeat_count(mod_ptr: *mut OpenMptModuleRaw, repeat_count: i32) -> c_int;
    fn openmpt_module_get_duration_seconds(mod_ptr: *mut OpenMptModuleRaw) -> c_double;
    fn openmpt_module_get_position_seconds(mod_ptr: *mut OpenMptModuleRaw) -> c_double;
    fn openmpt_module_set_position_seconds(
        mod_ptr: *mut OpenMptModuleRaw,
        seconds: c_double,
    ) -> c_double;

    fn openmpt_module_read_float_mono(
        mod_ptr: *mut OpenMptModuleRaw,
        samplerate: i32,
        count: usize,
        mono: *mut c_float,
    ) -> usize;

    fn openmpt_module_read_float_stereo(
        mod_ptr: *mut OpenMptModuleRaw,
        samplerate: i32,
        count: usize,
        left: *mut c_float,
        right: *mut c_float,
    ) -> usize;

    fn openmpt_module_read_interleaved_float_stereo(
        mod_ptr: *mut OpenMptModuleRaw,
        samplerate: i32,
        count: usize,
        interleaved_stereo: *mut c_float,
    ) -> usize;

    fn openmpt_module_read_float_quad(
        mod_ptr: *mut OpenMptModuleRaw,
        samplerate: i32,
        count: usize,
        left: *mut c_float,
        right: *mut c_float,
        rear_left: *mut c_float,
        rear_right: *mut c_float,
    ) -> usize;

    fn openmpt_module_read_interleaved_float_quad(
        mod_ptr: *mut OpenMptModuleRaw,
        samplerate: i32,
        count: usize,
        interleaved_quad: *mut c_float,
    ) -> usize;

    fn openmpt_module_get_metadata(
        mod_ptr: *mut OpenMptModuleRaw,
        key: *const c_char,
    ) -> *const c_char;

    fn openmpt_module_get_current_speed(mod_ptr: *mut OpenMptModuleRaw) -> i32;
    fn openmpt_module_get_current_tempo2(mod_ptr: *mut OpenMptModuleRaw) -> c_double;
    fn openmpt_module_get_current_order(mod_ptr: *mut OpenMptModuleRaw) -> i32;
    fn openmpt_module_get_current_row(mod_ptr: *mut OpenMptModuleRaw) -> i32;

    fn openmpt_module_get_current_channel_vu_mono(
        mod_ptr: *mut OpenMptModuleRaw,
        channel: i32,
    ) -> c_float;
    fn openmpt_module_get_current_channel_vu_left(
        mod_ptr: *mut OpenMptModuleRaw,
        channel: i32,
    ) -> c_float;
    fn openmpt_module_get_current_channel_vu_right(
        mod_ptr: *mut OpenMptModuleRaw,
        channel: i32,
    ) -> c_float;
    fn openmpt_module_get_current_channel_vu_rear_left(
        mod_ptr: *mut OpenMptModuleRaw,
        channel: i32,
    ) -> c_float;
    fn openmpt_module_get_current_channel_vu_rear_right(
        mod_ptr: *mut OpenMptModuleRaw,
        channel: i32,
    ) -> c_float;

    fn openmpt_module_get_num_channels(mod_ptr: *mut OpenMptModuleRaw) -> i32;
    fn openmpt_module_get_num_orders(mod_ptr: *mut OpenMptModuleRaw) -> i32;
    fn openmpt_module_get_num_patterns(mod_ptr: *mut OpenMptModuleRaw) -> i32;
    fn openmpt_module_get_num_instruments(mod_ptr: *mut OpenMptModuleRaw) -> i32;
    fn openmpt_module_get_num_samples(mod_ptr: *mut OpenMptModuleRaw) -> i32;

    fn openmpt_module_get_order_pattern(mod_ptr: *mut OpenMptModuleRaw, order: i32) -> i32;
    fn openmpt_module_get_pattern_num_rows(mod_ptr: *mut OpenMptModuleRaw, pattern: i32) -> i32;

    fn openmpt_module_format_pattern_row_channel(
        mod_ptr: *mut OpenMptModuleRaw,
        pattern: i32,
        row: i32,
        channel: i32,
        width: usize,
        pad: c_int,
    ) -> *const c_char;
}

/// Owned handle to a `libopenmpt` tracker module instance (`MOD`, `XM`, `S3M`, `IT`, `MPTM`, etc.).
///
/// Each `openmpt_module` instance in `libopenmpt` encapsulates independent state with no thread-local
/// affinity; `Send` is sound and `&mut self` guarantees exclusive access across calls.
pub struct OpenMptModule {
    ptr: NonNull<OpenMptModuleRaw>,
}

// SAFETY: `libopenmpt` documents that distinct `openmpt_module*` instances are completely
// independent and have no thread affinity. All methods require `&mut self`, preventing
// concurrent access to the same `openmpt_module*` without external synchronization.
unsafe impl Send for OpenMptModule {}

impl Drop for OpenMptModule {
    fn drop(&mut self) {
        // SAFETY: `self.ptr` is guaranteed valid and uniquely owned until drop.
        unsafe {
            openmpt_module_destroy(self.ptr.as_ptr());
        }
    }
}

impl OpenMptModule {
    /// Parses and initializes a tracker module from an in-memory byte slice.
    /// `libopenmpt` copies all required module data internally during construction.
    pub fn from_memory(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            bail!("Cannot load OpenMPT module from empty byte slice");
        }

        let mut err_code: c_int = 0;
        let mut err_msg_ptr: *const c_char = ptr::null();

        // SAFETY: `data.as_ptr()` is valid for `data.len()` bytes during the synchronous call;
        // `openmpt_log_func_silent` and `openmpt_error_func_ignore` are `libopenmpt`'s built-in
        // thread-safe callbacks; any allocated `err_msg_ptr` is freed via `openmpt_free_string`.
        let raw = unsafe {
            openmpt_module_create_from_memory2(
                data.as_ptr().cast::<c_void>(),
                data.len(),
                Some(openmpt_log_func_silent),
                ptr::null_mut(),
                Some(openmpt_error_func_ignore),
                ptr::null_mut(),
                &mut err_code,
                &mut err_msg_ptr,
                ptr::null(),
            )
        };

        let err_detail = if !err_msg_ptr.is_null() {
            // SAFETY: `err_msg_ptr` was allocated by `libopenmpt` as a NUL-terminated UTF-8 string.
            let msg = unsafe { CStr::from_ptr(err_msg_ptr).to_string_lossy().into_owned() };
            unsafe {
                openmpt_free_string(err_msg_ptr);
            }
            msg
        } else {
            String::new()
        };

        let Some(ptr) = NonNull::new(raw) else {
            if err_detail.is_empty() {
                bail!("libopenmpt failed to load module (error code {err_code})");
            } else {
                bail!("libopenmpt failed to load module (error code {err_code}): {err_detail}");
            }
        };

        Ok(Self { ptr })
    }

    /// Renders mono `f32` frames directly into `mono` with zero heap allocations.
    /// Returns the number of frames rendered (`0` on end-of-module or empty slice).
    #[inline]
    pub fn read_float_mono(&mut self, sample_rate: i32, mono: &mut [f32]) -> usize {
        if mono.is_empty() || sample_rate <= 0 {
            return 0;
        }
        // SAFETY: `mono` has at least `mono.len()` contiguous writable `f32` elements.
        unsafe {
            openmpt_module_read_float_mono(
                self.ptr.as_ptr(),
                sample_rate,
                mono.len(),
                mono.as_mut_ptr(),
            )
        }
    }

    /// Renders planar stereo `f32` frames directly into `left` and `right` with zero heap allocations.
    /// Uses `left.len().min(right.len())` as the exact frame bound.
    #[inline]
    pub fn read_float_stereo(
        &mut self,
        sample_rate: i32,
        left: &mut [f32],
        right: &mut [f32],
    ) -> usize {
        let count = left.len().min(right.len());
        if count == 0 || sample_rate <= 0 {
            return 0;
        }
        // SAFETY: Both `left` and `right` have at least `count` contiguous writable `f32` elements.
        unsafe {
            openmpt_module_read_float_stereo(
                self.ptr.as_ptr(),
                sample_rate,
                count,
                left.as_mut_ptr(),
                right.as_mut_ptr(),
            )
        }
    }

    /// Renders interleaved `(L, R)` stereo `f32` frames directly into `interleaved_stereo`
    /// (`interleaved_stereo.len() / 2` max frames) with zero heap allocations.
    #[inline]
    pub fn read_interleaved_float_stereo(
        &mut self,
        sample_rate: i32,
        interleaved_stereo: &mut [f32],
    ) -> usize {
        let count = interleaved_stereo.len() / 2;
        if count == 0 || sample_rate <= 0 {
            return 0;
        }
        // SAFETY: `interleaved_stereo` has at least `count * 2` contiguous writable `f32` elements.
        unsafe {
            openmpt_module_read_interleaved_float_stereo(
                self.ptr.as_ptr(),
                sample_rate,
                count,
                interleaved_stereo.as_mut_ptr(),
            )
        }
    }

    /// Renders planar quadraphonic `(L, R, RL, RR)` `f32` frames directly into caller slices
    /// with zero heap allocations, bounded by the shortest plane slice.
    #[inline]
    pub fn read_float_quad(
        &mut self,
        sample_rate: i32,
        left: &mut [f32],
        right: &mut [f32],
        rear_left: &mut [f32],
        rear_right: &mut [f32],
    ) -> usize {
        let count = left
            .len()
            .min(right.len())
            .min(rear_left.len())
            .min(rear_right.len());
        if count == 0 || sample_rate <= 0 {
            return 0;
        }
        // SAFETY: All four plane slices have at least `count` contiguous writable `f32` elements.
        unsafe {
            openmpt_module_read_float_quad(
                self.ptr.as_ptr(),
                sample_rate,
                count,
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                rear_left.as_mut_ptr(),
                rear_right.as_mut_ptr(),
            )
        }
    }

    /// Renders interleaved `(L, R, RL, RR)` quadraphonic `f32` frames directly into `interleaved_quad`
    /// (`interleaved_quad.len() / 4` max frames) with zero heap allocations.
    #[inline]
    pub fn read_interleaved_float_quad(
        &mut self,
        sample_rate: i32,
        interleaved_quad: &mut [f32],
    ) -> usize {
        let count = interleaved_quad.len() / 4;
        if count == 0 || sample_rate <= 0 {
            return 0;
        }
        // SAFETY: `interleaved_quad` has at least `count * 4` contiguous writable `f32` elements.
        unsafe {
            openmpt_module_read_interleaved_float_quad(
                self.ptr.as_ptr(),
                sample_rate,
                count,
                interleaved_quad.as_mut_ptr(),
            )
        }
    }

    #[inline]
    pub fn set_repeat_count(&mut self, repeat_count: i32) -> bool {
        unsafe { openmpt_module_set_repeat_count(self.ptr.as_ptr(), repeat_count) != 0 }
    }

    #[inline]
    pub fn duration_seconds(&mut self) -> f64 {
        unsafe { openmpt_module_get_duration_seconds(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn position_seconds(&mut self) -> f64 {
        unsafe { openmpt_module_get_position_seconds(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn set_position_seconds(&mut self, seconds: f64) -> f64 {
        unsafe { openmpt_module_set_position_seconds(self.ptr.as_ptr(), seconds) }
    }

    #[inline]
    pub fn current_channel_vu_mono(&mut self, channel: i32) -> f32 {
        unsafe { openmpt_module_get_current_channel_vu_mono(self.ptr.as_ptr(), channel) }
    }

    #[inline]
    pub fn current_channel_vu_stereo(&mut self, channel: i32) -> (f32, f32) {
        unsafe {
            let l = openmpt_module_get_current_channel_vu_left(self.ptr.as_ptr(), channel);
            let r = openmpt_module_get_current_channel_vu_right(self.ptr.as_ptr(), channel);
            (l, r)
        }
    }

    #[inline]
    pub fn current_channel_vu_quad(&mut self, channel: i32) -> [f32; 4] {
        unsafe {
            [
                openmpt_module_get_current_channel_vu_left(self.ptr.as_ptr(), channel),
                openmpt_module_get_current_channel_vu_right(self.ptr.as_ptr(), channel),
                openmpt_module_get_current_channel_vu_rear_left(self.ptr.as_ptr(), channel),
                openmpt_module_get_current_channel_vu_rear_right(self.ptr.as_ptr(), channel),
            ]
        }
    }

    /// Queries a metadata string (`c"artist"`, `c"title"`, `c"type"`, `c"tracker"`, etc.).
    /// Returns `None` if the metadata key is unset or empty.
    pub fn metadata(&mut self, key: &CStr) -> Option<String> {
        // SAFETY: `key.as_ptr()` is a valid NUL-terminated string; returned pointer (if non-null)
        // is owned by the caller and freed via `openmpt_free_string`.
        unsafe {
            let raw = openmpt_module_get_metadata(self.ptr.as_ptr(), key.as_ptr());
            if raw.is_null() {
                return None;
            }
            let s = CStr::from_ptr(raw).to_string_lossy();
            let trimmed = s.trim();
            let out = if trimmed.is_empty() {
                None
            } else {
                Some(s.into_owned())
            };
            openmpt_free_string(raw);
            out
        }
    }

    #[inline]
    pub fn current_tempo(&mut self) -> i32 {
        unsafe { openmpt_module_get_current_tempo2(self.ptr.as_ptr()).round() as i32 }
    }

    #[inline]
    pub fn current_speed(&mut self) -> i32 {
        unsafe { openmpt_module_get_current_speed(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn current_order(&mut self) -> i32 {
        unsafe { openmpt_module_get_current_order(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn current_row(&mut self) -> i32 {
        unsafe { openmpt_module_get_current_row(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn num_channels(&mut self) -> i32 {
        unsafe { openmpt_module_get_num_channels(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn num_orders(&mut self) -> i32 {
        unsafe { openmpt_module_get_num_orders(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn num_patterns(&mut self) -> i32 {
        unsafe { openmpt_module_get_num_patterns(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn num_instruments(&mut self) -> i32 {
        unsafe { openmpt_module_get_num_instruments(self.ptr.as_ptr()) }
    }

    #[inline]
    pub fn num_samples(&mut self) -> i32 {
        unsafe { openmpt_module_get_num_samples(self.ptr.as_ptr()) }
    }

    /// Returns `Some(pattern_idx)` if `order` maps to a valid playable pattern with `> 0` rows,
    /// or `None` for out-of-range, `+++` skip, or `---` stop order entries.
    #[inline]
    pub fn playable_order_pattern(&mut self, order: i32) -> Option<i32> {
        let num_pats = self.num_patterns();
        let pat = unsafe { openmpt_module_get_order_pattern(self.ptr.as_ptr(), order) };
        if pat < 0 || pat >= num_pats {
            return None;
        }
        if self.pattern_num_rows(pat) <= 0 {
            return None;
        }
        Some(pat)
    }

    #[inline]
    pub fn pattern_num_rows(&mut self, pattern: i32) -> i32 {
        unsafe { openmpt_module_get_pattern_num_rows(self.ptr.as_ptr(), pattern) }
    }

    /// Appends the formatted `"ch0 | ch1 | ..."` pattern row into `out` without allocating
    /// per-cell Rust `String`s, freeing each C string via `openmpt_free_string`.
    pub fn format_pattern_row_into(
        &mut self,
        pattern: i32,
        row: i32,
        num_channels: i32,
        out: &mut String,
    ) {
        out.clear();
        let num_rows = self.pattern_num_rows(pattern);
        if row < 0 || row >= num_rows || num_channels <= 0 {
            return;
        }
        for c in 0..num_channels {
            // SAFETY: `self.ptr` is valid; returned string is freed via `openmpt_free_string`.
            unsafe {
                let cell_ptr = openmpt_module_format_pattern_row_channel(
                    self.ptr.as_ptr(),
                    pattern,
                    row,
                    c,
                    0,
                    0,
                );
                if !cell_ptr.is_null() {
                    if c != 0 {
                        out.push_str(" | ");
                    }
                    out.push_str(&CStr::from_ptr(cell_ptr).to_string_lossy());
                    openmpt_free_string(cell_ptr);
                }
            }
        }
    }

    /// Pre-formats all orders and rows of the tracker module for the Pattern View UI.
    pub fn pre_format_all_patterns(&mut self) -> Vec<Vec<String>> {
        let num_orders = self.num_orders().max(0);
        let num_channels = self.num_channels().max(0);
        let mut patterns_by_order = Vec::with_capacity(num_orders as usize);

        for o in 0..num_orders {
            let mut row_strings = Vec::new();
            if let Some(pat) = self.playable_order_pattern(o) {
                let num_rows = self.pattern_num_rows(pat).max(0);
                row_strings.reserve(num_rows as usize);
                for r in 0..num_rows {
                    let mut row_str = String::with_capacity((num_channels as usize) * 16);
                    self.format_pattern_row_into(pat, r, num_channels, &mut row_str);
                    row_strings.push(row_str);
                }
            }
            patterns_by_order.push(row_strings);
        }

        patterns_by_order
    }

    /// Formats the currently playing pattern row for HUD/TUI display.
    pub fn current_row_string(&mut self) -> String {
        let num_channels = self.num_channels();
        let cur_order = self.current_order();
        let cur_row = self.current_row();
        let mut out = String::new();
        if let Some(pat) = self.playable_order_pattern(cur_order) {
            self.format_pattern_row_into(pat, cur_row, num_channels, &mut out);
        }
        out
    }
}

/// Builds a self-contained, valid 4-channel ProTracker (`M.K.`) `.MOD` module in memory
/// with a non-silent square-wave sample and C-5 note trigger on row 0 channel 0.
#[cfg(test)]
pub(crate) fn build_minimal_protracker_mod() -> Vec<u8> {
    // 1084-byte header + 1 pattern (64 rows * 4 ch * 4 B = 1024 B) + 32-byte sample = 2140 B
    let mut mod_bytes = vec![0u8; 1084 + 1024 + 32];
    // Song title (20 bytes)
    let title = b"RustTracker FFI Test";
    mod_bytes[..title.len()].copy_from_slice(title);

    // Sample 1 header starts at byte 20 (30 bytes per sample):
    // [0..22] name, [22..24] length in 16-bit words (big-endian: 16 words = 32 bytes),
    // [24] finetune = 0, [25] volume = 64, [26..28] repeat offset = 0, [28..30] repeat len = 16 words
    mod_bytes[20..31].copy_from_slice(b"SquareLead\0");
    mod_bytes[42..44].copy_from_slice(&16u16.to_be_bytes());
    mod_bytes[45] = 64; // max volume
    mod_bytes[48..50].copy_from_slice(&16u16.to_be_bytes()); // loop entire 32-byte sample

    // Song length = 1 order at offset 950, restart = 127 at 951, order[0] = pattern 0 at 952
    mod_bytes[950] = 1;
    mod_bytes[951] = 127;
    mod_bytes[952] = 0;
    // Magic "M.K." at offset 1080..1084
    mod_bytes[1080..1084].copy_from_slice(b"M.K.");

    // Pattern 0, row 0, channel 0 at offset 1084..1088:
    // Sample 1, period 428 (C-2 / C-5), no effect:
    // byte0 = (sample & 0xF0) | ((period >> 8) & 0x0F)
    // byte1 = (period & 0xFF)
    // byte2 = ((sample & 0x0F) << 4) | (effect & 0x0F)
    // byte3 = effect_param
    let sample_num = 1u16;
    let period = 428u16;
    mod_bytes[1084] = ((sample_num as u8) & 0xF0) | (((period >> 8) & 0x0F) as u8);
    mod_bytes[1085] = (period & 0xFF) as u8;
    mod_bytes[1086] = ((sample_num as u8) & 0x0F) << 4;
    mod_bytes[1087] = 0;

    // Sample 1 PCM data (signed 8-bit square wave, 32 bytes at offset 2108..2140)
    // Note: first 2 bytes of MOD sample 1 are cleared by some classic players unless looped,
    // so fill with non-zero +/-96 square wave after the initial loop word.
    for i in 0..32 {
        mod_bytes[2108 + i] = if i < 16 { 96i8 as u8 } else { (-96i8) as u8 };
    }
    mod_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openmpt_ffi_rejects_empty_and_corrupt_buffers() {
        assert!(OpenMptModule::from_memory(&[]).is_err());
        assert!(OpenMptModule::from_memory(b"not a valid tracker file").is_err());
        assert!(OpenMptModule::from_memory(&[0xFF; 256]).is_err());
    }

    #[test]
    fn test_openmpt_ffi_zero_alloc_synthetic_mod_decode_and_patterns() {
        let mod_data = build_minimal_protracker_mod();
        let mut module = OpenMptModule::from_memory(&mod_data)
            .expect("synthetic M.K. ProTracker MOD must load cleanly via libopenmpt C FFI");

        assert_eq!(module.num_channels(), 4);
        assert_eq!(module.num_orders(), 1);
        assert_eq!(module.num_patterns(), 1);
        assert!(module.num_samples() >= 1);
        assert!(module.duration_seconds() > 0.5);
        assert_eq!(
            module.metadata(c"type").as_deref(),
            Some("mod"),
            "MOD format extension metadata must report 'mod'"
        );
        assert_eq!(
            module.metadata(c"title").as_deref(),
            Some("RustTracker FFI Test")
        );

        let patterns = module.pre_format_all_patterns();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].len(), 64);
        assert!(
            patterns[0][0].contains("C-5") || patterns[0][0].contains("C-2"),
            "Row 0 channel 0 must format note trigger, got: {}",
            patterns[0][0]
        );
        assert!(
            patterns[0][0].contains(" | "),
            "Multi-channel row must include ' | ' separators"
        );

        // Verify boundary/empty slice handling without panics or writes past slice len
        let mut empty: [f32; 0] = [];
        assert_eq!(module.read_float_mono(48_000, &mut empty), 0);
        assert_eq!(module.read_interleaved_float_stereo(48_000, &mut empty), 0);
        let mut odd_single = [0.0f32; 1];
        assert_eq!(
            module.read_interleaved_float_stereo(48_000, &mut odd_single),
            0
        );
        assert_eq!(module.read_float_mono(0, &mut odd_single), 0);

        // Render interleaved stereo into a fixed stack array with guard sentinels at the tail
        let mut buf = [f32::NAN; 1024 + 4];
        let ptr_before = buf.as_ptr();
        let frames = module.read_interleaved_float_stereo(48_000, &mut buf[..1024]);
        assert_eq!(frames, 512);
        assert_eq!(buf.as_ptr(), ptr_before);
        assert!(
            buf[..1024].iter().all(|s| s.is_finite()),
            "All rendered stereo samples must be finite"
        );
        assert!(
            buf[..1024].iter().any(|s| s.abs() > 0.01),
            "Active square-wave note must produce non-zero audio amplitude"
        );
        // Guard sentinels past slice end MUST remain untouched (unlike old openmpt crate capacity bug!)
        assert!(buf[1024..].iter().all(|s| s.is_nan()));

        // Verify VU meter, tempo/speed, and seek roundtrip
        let (vu_l, vu_r) = module.current_channel_vu_stereo(0);
        assert!(vu_l >= 0.0 && vu_r >= 0.0);
        assert!(module.current_channel_vu_mono(0) > 0.0);
        assert!(module.current_tempo() > 0);
        assert!(module.current_speed() > 0);

        let seeked = module.set_position_seconds(0.0);
        assert!((0.0..0.05).contains(&seeked));

        let mut left = [0.0f32; 256];
        let mut right = [0.0f32; 128]; // Deliberately asymmetric to verify min(left, right) bound
        let planar_frames = module.read_float_stereo(48_000, &mut left, &mut right);
        assert_eq!(planar_frames, 128);

        // Verify zero-allocation quadraphonic FFI bounds & tail sentinel safety
        let mut quad_interleaved = [f32::NAN; 512 + 4];
        let quad_frames = module.read_interleaved_float_quad(48_000, &mut quad_interleaved[..512]);
        assert_eq!(quad_frames, 128);
        assert!(quad_interleaved[..512].iter().all(|s| s.is_finite()));
        assert!(quad_interleaved[512..].iter().all(|s| s.is_nan()));

        let mut rl = [0.0f32; 64];
        let mut rr = [0.0f32; 96];
        let planar_quad = module.read_float_quad(48_000, &mut left, &mut right, &mut rl, &mut rr);
        assert_eq!(planar_quad, 64);
        let vu_quad = module.current_channel_vu_quad(0);
        assert!(vu_quad.iter().all(|v| v.is_finite() && *v >= 0.0));
    }

    #[test]
    fn test_openmpt_ffi_real_xm_and_mod_fixtures_if_present() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for rel in ["audio_tests/uc-wof.xm", "audio_tests/electric_drug.mod"] {
            let path = root.join(rel);
            if !path.exists() {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read tracker fixture");
            let mut module = OpenMptModule::from_memory(&bytes)
                .unwrap_or_else(|e| panic!("Failed to decode {}: {e}", path.display()));
            assert!(module.duration_seconds() > 1.0);
            assert!(module.num_channels() > 0);
            assert!(module.num_patterns() > 0);

            let mut stereo = [0.0f32; 4096];
            let mut peak = 0.0f32;
            for _ in 0..8 {
                let n = module.read_interleaved_float_stereo(48_000, &mut stereo);
                assert!(n > 0);
                for &s in &stereo[..n * 2] {
                    assert!(s.is_finite());
                    peak = peak.max(s.abs());
                }
            }
            assert!(
                peak > 1e-4,
                "Real tracker fixture {} produced silence",
                path.display()
            );
        }
    }
}
