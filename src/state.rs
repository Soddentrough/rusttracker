use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32};

pub static UNCAPPED_PRESENT_MODE: AtomicBool = AtomicBool::new(false);
pub static BENCH_FRAMES_OVERRIDE: AtomicU32 = AtomicU32::new(0);
pub static CACHE_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamepadType {
    Xbox,
    PlayStation,
    Nintendo,
    SteamDeck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MobileHudTab {
    #[default]
    Channels,
    Heatmap,
    Info,
    Video,
}

impl MobileHudTab {
    pub fn next(&self, has_video: bool) -> Self {
        match self {
            Self::Channels => Self::Heatmap,
            Self::Heatmap => Self::Info,
            Self::Info => {
                if has_video {
                    Self::Video
                } else {
                    Self::Channels
                }
            }
            Self::Video => Self::Channels,
        }
    }

    pub fn prev(&self, has_video: bool) -> Self {
        match self {
            Self::Channels => {
                if has_video {
                    Self::Video
                } else {
                    Self::Info
                }
            }
            Self::Heatmap => Self::Channels,
            Self::Info => Self::Heatmap,
            Self::Video => Self::Info,
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct PerformanceStats {
    pub decode_us: f32,
    pub fft_us: f32,
    pub ui_us: f32,
    pub render_us: f32,
    pub shader_us: f32,
    pub fire_us: f32,
    pub gpu_fft_us: f32,
    pub audio_buffer_fill_pct: f32,
    pub video_buffer_fill_pct: f32,
    pub clipping_events: u32,
    pub bitstream_active: bool,
    pub bitstream_format: String,
    pub mpegh_bitstream_active: bool,
    pub mpegh_carrier_info: String,
    pub video_hw_decoder: String,
    pub video_fps: f32,
    pub video_bitrate_kbps: Option<u32>,
    pub audio_bitrate_kbps: Option<u32>,

    // Fine-grained frame phase timings (microseconds)
    pub phase_lock_update_us: f32, // Main loop: lock + smoothing + engine.update()
    pub phase_snapshot_us: f32,    // Main loop: render_snapshot() creation
    pub phase_surface_us: f32,     // render(): get_current_texture()
    pub phase_egui_layout_us: f32, // render(): egui ctx.run() UI layout
    pub phase_encode_us: f32,      // render(): GPU command encoding (compute + render passes)
    pub phase_submit_us: f32,      // render(): queue.submit() + present
    pub phase_post_us: f32,        // Main loop: post-render state writeback
}

#[derive(Clone)]
pub struct VideoFrame {
    pub pts: f64,
    pub width: u32,
    pub height: u32,
    pub rotation: u32,
    pub y_plane: Vec<u8>,
    pub u_plane: Vec<u8>,
    pub v_plane: Vec<u8>,
    pub y_stride: usize,
    pub u_stride: usize,
    pub v_stride: usize,

    // HDR Metadata
    pub bit_depth: u8,
    pub color_space: u32,
    pub color_range: u32,
    pub color_trc: u32,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("pts", &self.pts)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rotation", &self.rotation)
            .field("bit_depth", &self.bit_depth)
            .field("color_space", &self.color_space)
            .field("color_range", &self.color_range)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Geometry {
    Grid { width: u32, depth: u32 },
    UnitQuad,
    UnitBox,
    NeonRoom,
    SynthwaveRacerScene,
    VuMeterRack,
    GlassLyricsScene,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineType {
    FullscreenQuad,
    Mesh3D { geometry: Geometry, instances: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuCapabilityTier {
    #[default]
    High,
    Constrained,
}

/// Evaluates GPU hardware capability and display pixel load in an OS-agnostic manner.
///
/// Rather than disabling visualizations based on the operating system (e.g. Android runs
/// on everything from entry-level tablets to flagship Snapdragon 8 Elite / Immortalis phones
/// that outperform many laptops), this inspects the Vulkan/Metal/DX12 adapter architecture
/// and physical viewport resolution to determine whether heavy per-pixel raymarching / 3D FLIP
/// hydrodynamic shaders should be enabled by default in rotation.
pub fn classify_gpu_capability(
    info: &wgpu::AdapterInfo,
    viewport_pixels: u32,
) -> GpuCapabilityTier {
    match info.device_type {
        wgpu::DeviceType::DiscreteGpu => GpuCapabilityTier::High,
        wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu => GpuCapabilityTier::Constrained,
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::Other => {
            let name = info.name.to_lowercase();

            // Software / fallback renderers reporting as Integrated/Other
            if name.contains("llvmpipe")
                || name.contains("lavapipe")
                || name.contains("swiftshader")
                || name.contains(" microsoft basic render")
            {
                return GpuCapabilityTier::Constrained;
            }

            // Steam Deck custom Van Gogh APU (15W TDP shared budget)
            if name.contains("custom gpu 0405")
                || name.contains("custom gpu 0932")
                || name.contains("vangogh")
            {
                return GpuCapabilityTier::Constrained;
            }

            // Standard ARM Mali-G / Mali-T series (non-Immortalis, e.g. Mali-G710 on Tensor G2)
            if (name.contains("mali-g") || name.contains("mali-t")) && !name.contains("immortalis")
            {
                return GpuCapabilityTier::Constrained;
            }

            // Qualcomm Adreno: Adreno 8xx and Adreno 730/740/750+ are flagship High tier;
            // older Adreno 4xx/5xx/6xx and mid-range Adreno 702/710/720/725 are Constrained.
            if name.contains("adreno") {
                if name.contains("adreno (tm) 8")
                    || name.contains("adreno 8")
                    || name.contains("adreno (tm) 73")
                    || name.contains("adreno 73")
                    || name.contains("adreno (tm) 74")
                    || name.contains("adreno 74")
                    || name.contains("adreno (tm) 75")
                    || name.contains("adreno 75")
                    || name.contains("adreno (tm) x")
                    || name.contains("adreno x")
                {
                    return GpuCapabilityTier::High;
                }
                return GpuCapabilityTier::Constrained;
            }

            // Legacy Intel HD / UHD integrated graphics
            if name.contains("hd graphics") || name.contains("uhd graphics") {
                return GpuCapabilityTier::Constrained;
            }

            // Known high-tier integrated/SoC architectures (Apple Silicon, ARM Immortalis,
            // Samsung Xclipse 9xx, AMD RDNA2/3/3.5 680M/780M/880M/890M/80xxS, Intel Arc/Iris Xe)
            if name.contains("apple")
                || name.contains("immortalis")
                || name.contains("xclipse 9")
                || name.contains("radeon 680m")
                || name.contains("radeon 780m")
                || name.contains("radeon 880m")
                || name.contains("radeon 890m")
                || name.contains("radeon 80")
                || name.contains("intel(r) arc")
                || name.contains("iris(r) xe")
            {
                return GpuCapabilityTier::High;
            }

            // Unknown integrated GPU driving a high-DPI (> 3.6 MP, e.g. 2560x1600 QHD+) panel
            if viewport_pixels >= 3_600_000 {
                return GpuCapabilityTier::Constrained;
            }

            GpuCapabilityTier::High
        }
    }
}

#[derive(Clone, Debug)]
pub struct VisualizerDef {
    pub id: u32,
    pub name: &'static str,
    pub filename: &'static str,
    pub description: &'static str,
    pub pipeline_type: PipelineType,
    pub requires_history: bool,
    pub requires_fire: bool,
    pub requires_resynth: bool,
    pub requires_ferrofluidsim: bool,
    pub requires_flip_ferrofluid: bool,
    pub requires_flip_water: bool,
    /// True for compute/fillrate-heavy shaders (per-pixel raymarching, multi-octave FBM,
    /// or 3D FLIP hydrodynamic compute + heightfield raymarching) that require a High-tier
    /// GPU to sustain 60+ FPS at high resolutions.
    pub heavy_shader: bool,
}

pub const VISUALIZERS: &[VisualizerDef] = &[
    VisualizerDef {
        id: 0,
        name: "Frequency Spectrum",
        filename: "vis_spectrum.wgsl",
        description: "Standard 2D FFT spectrum analyzer",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 25,
        name: "Studio LED Spectrum",
        filename: "vis_spectrum_led.wgsl",
        description: "Segmented studio LED frequency spectrum with peak hold",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 24,
        name: "Retro Tape Head",
        filename: "vis_tape_head.wgsl",
        description: "Optical playhead with cathode spark impact & streaming waveform",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 1,
        name: "CRT Oscilloscope",
        filename: "vis_oscilloscope.wgsl",
        description: "2D glowing CRT wave trace",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: true,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 3,
        name: "3D Oscilloscope",
        filename: "vis_3doscilloscope_raster.wgsl",
        description: "Rasterized 3D waterfall grid of waveform history",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::Grid {
                width: 160,
                depth: 95,
            },
            instances: 1,
        },
        requires_history: true,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 4,
        name: "Strings",
        filename: "vis_3doscilloscope_freq.wgsl",
        description: "3D stereo vibrating frequency strings",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::Grid {
                width: 511,
                depth: 1,
            },
            instances: 32,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: true,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 5,
        name: "Retro Fire",
        filename: "vis_flame.wgsl",
        description: "Classic DOOM 2D fire simulation with CRT post-processing",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: true,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 6,
        name: "Fire Simulation",
        filename: "vis_firesim.wgsl",
        description: "Multi-channel procedural fire simulation (experimental)",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: true,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 7,
        name: "Solar Flare",
        filename: "vis_solar.wgsl",
        description: "Audio-reactive raymarched sun",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 13,
        name: "Synthwave Terrain",
        filename: "vis_synthwave.wgsl",
        description: "Retro 80s wireframe landscape",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::Grid {
                width: 120,
                depth: 120,
            },
            instances: 1,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 14,
        name: "Synthwave 3D Racer",
        filename: "vis_synthwave_racer_3d.wgsl",
        description: "Multi-pass 3D low-poly sports car, highway & infinite horizon sky",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::SynthwaveRacerScene,
            instances: 1,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 16,
        name: "Midnight Storm",
        filename: "vis_rain.wgsl",
        description: "Cinematic atmospheric dark sky, falling rain & branching lightning",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 19,
        name: "Retro VU Meters",
        filename: "vis_vumeters_3d.wgsl",
        description: "3D vintage Hi-Fi master rack with physical analog needles & warm glow",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::VuMeterRack,
            instances: 1,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 20,
        name: "Bioluminescent Waves",
        filename: "vis_bioluminescence.wgsl",
        description: "GPU compute-driven bioluminescent particle waves",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::UnitQuad,
            instances: 36864,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 22,
        name: "Neon Spatial Room",
        filename: "vis_neon_room.wgsl",
        description: "3D interactive spatial listening room with multi-channel speakers",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::NeonRoom,
            instances: 1,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
    VisualizerDef {
        id: 23,
        name: "3D Glass Water Lyrics",
        filename: "vis_lyrics.wgsl",
        description: "Chunky 3D glass letters in dark reflective water with kinetic slam & splashes",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::GlassLyricsScene,
            instances: 1,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: true,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 27,
        name: "Quantum FLIP Ferrofluid",
        filename: "vis_flip_ferrofluid.wgsl",
        description: "Incompressible FLIP hydrodynamic simulation with Rosensweig magnetic spikes",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: true,
        requires_flip_water: false,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 28,
        name: "Acoustic Glass Water Chamber",
        filename: "vis_glass_water.wgsl",
        description: "Expansive FLIP acoustic ocean with swells, Faraday ripples & seabed caustics",
        pipeline_type: PipelineType::FullscreenQuad,
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: true,
        heavy_shader: true,
    },
    VisualizerDef {
        id: 29,
        name: "Prismatic Spectrum Canyon",
        filename: "vis_spectrum_canyon.wgsl",
        description: "3D mirrored perspective rainbow spectrum canyon with segmented LED bars",
        pipeline_type: PipelineType::Mesh3D {
            geometry: Geometry::UnitBox,
            instances: 8193,
        },
        requires_history: false,
        requires_fire: false,
        requires_resynth: false,
        requires_ferrofluidsim: false,
        requires_flip_ferrofluid: false,
        requires_flip_water: false,
        heavy_shader: false,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct AudioTrackInfo {
    pub id: usize,
    pub title: String,
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub language: Option<String>,
}

/// Configuration for Automatic Surround & Height Upmixing and the 5-Band Crossover EQ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurroundUpmixConfig {
    /// Whether automatic surround & height upmixing is active.
    pub enabled: bool,
    /// Whether the user has manually toggled or overridden `enabled`.
    pub user_overridden: bool,
    /// Whether to encode 7.1.4 (11.1, CICP Layout 19) output as an MPEG-H 3D Audio IEC 61937-13
    /// bitstream over HDMI when 12-channel LPCM is capped at 8 channels by HDMI hardware.
    pub mpegh_bitstream: bool,
    /// Whether the user has manually toggled or overridden `mpegh_bitstream`.
    pub mpegh_user_overridden: bool,
    /// Low-pass Linkwitz-Riley crossover frequency (Hz) for LFE subwoofer extraction (40..=250 Hz).
    pub sub_crossover_hz: f32,
    /// High-pass Linkwitz-Riley crossover frequency (Hz) splitting highs up to height speakers (800..=10000 Hz).
    pub height_crossover_hz: f32,
    /// 5-band parametric EQ gains in dB (-12.0..=12.0 dB) at center frequencies [60, 250, 1000, 3500, 10000] Hz.
    pub band_gains_db: [f32; 5],
    /// Front L/R speaker level weight (0.0..=1.5).
    pub front_level: f32,
    /// Side & Rear L/R bed surround speaker level weight (0.0..=1.5).
    pub side_rear_level: f32,
    /// Height (Top Front / Top Rear) L/R speaker level weight (0.0..=1.5).
    pub height_level: f32,
    /// Center speaker mid-extraction level weight (0.0..=1.5; 0.0 = pure Left/Right side-only).
    pub center_level: f32,
    /// LFE subwoofer level weight (0.0..=1.5).
    pub lfe_level: f32,
    /// Fraction of high-frequency band (> height_crossover_hz) kept in ear-level Side/Rear speakers (0.0..=1.0).
    pub surround_high_bleed: f32,
    /// Mid/Side ambient extraction boost for rear and height speakers (0.0..=2.0).
    pub stereo_width: f32,
}

impl Default for SurroundUpmixConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            user_overridden: false,
            mpegh_bitstream: true,
            mpegh_user_overridden: false,
            sub_crossover_hz: 90.0,
            height_crossover_hz: 2800.0,
            band_gains_db: [0.0; 5],
            front_level: 1.0,
            side_rear_level: 0.85,
            height_level: 0.95,
            center_level: 0.45,
            lfe_level: 0.85,
            surround_high_bleed: 0.25,
            stereo_width: 1.0,
        }
    }
}

impl SurroundUpmixConfig {
    pub const EQ_BAND_FREQS: [f32; 5] = [60.0, 250.0, 1000.0, 3500.0, 10000.0];
    pub const EQ_BAND_LABELS: [&'static str; 5] = ["60Hz", "250Hz", "1kHz", "3.5kHz", "10kHz"];

    /// Reset EQ bands, crossover cutoffs, and speaker levels back to factory defaults,
    /// preserving the current `enabled` and `mpegh_bitstream` states.
    pub fn reset_tuning(&mut self) {
        let user_overridden = self.user_overridden;
        let enabled = self.enabled;
        let mpegh_bitstream = self.mpegh_bitstream;
        let mpegh_user_overridden = self.mpegh_user_overridden;
        *self = Self::default();
        self.enabled = enabled;
        self.user_overridden = user_overridden;
        self.mpegh_bitstream = mpegh_bitstream;
        self.mpegh_user_overridden = mpegh_user_overridden;
    }

    /// Evaluate the composite 5-band EQ gain in dB at a given frequency `freq_hz`.
    pub fn evaluate_eq_db(&self, freq_hz: f32) -> f32 {
        let f = freq_hz.clamp(20.0, 20000.0);
        let log_f = f.log2();
        let mut total_db = 0.0f32;

        for (i, &center_f) in Self::EQ_BAND_FREQS.iter().enumerate() {
            let gain_db = self.band_gains_db[i].clamp(-12.0, 12.0);
            if gain_db.abs() < 1e-4 {
                continue;
            }
            let log_c = center_f.log2();
            let oct_dist = log_f - log_c;
            let weight = if i == 0 {
                // Low-shelf with gentle bell transition above 60 Hz
                if oct_dist <= 0.0 {
                    1.0
                } else {
                    (-oct_dist * oct_dist * 1.15).exp()
                }
            } else if i == 4 {
                // High-shelf with gentle bell transition below 10 kHz
                if oct_dist >= 0.0 {
                    1.0
                } else {
                    (-oct_dist * oct_dist * 1.15).exp()
                }
            } else {
                // Parametric peaking bell (~1.1 octave bandwidth)
                (-oct_dist * oct_dist * 1.35).exp()
            };
            total_db += gain_db * weight;
        }
        total_db.clamp(-15.0, 15.0)
    }

    /// Returns `true` when surround upmixing is enabled and the output hardware device
    /// has a multi-channel surround layout (`> 2` channels). On stereo/mono devices (`<= 2`),
    /// surround upmixing is bypassed so audio and channel counts remain untouched.
    #[inline]
    pub fn is_active_for_hardware(&self, hardware_channels: i32) -> bool {
        self.enabled && hardware_channels > 2
    }

    /// Returns `true` when surround upmixing should be enabled by default for a media file
    /// with `source_channels` playing on an output device with `device_channels`:
    /// enabled for all cases on multi-channel output hardware (`device_channels > 2`)
    /// except where the media file's channel count matches the output device (`source_channels == device_channels`).
    #[inline]
    pub fn should_enable_by_default(source_channels: i32, device_channels: i32) -> bool {
        device_channels > 2 && source_channels > 0 && source_channels < device_channels
    }

    /// Returns `true` when surround upmixing is applicable for a source with `source_channels`
    /// on an output device with `hardware_channels`. When the source already has at least as many
    /// discrete channels as the output hardware (`source_channels >= hardware_channels`), upmixing
    /// is not applicable because all output channels are already driven by discrete source stems.
    #[inline]
    pub fn can_upmix(source_channels: i32, hardware_channels: i32) -> bool {
        hardware_channels > 2 && (source_channels <= 0 || source_channels < hardware_channels)
    }

    /// Returns `true` when surround upmixing is both enabled and applicable for `(source_channels, hardware_channels)`.
    #[inline]
    pub fn is_active_for(&self, source_channels: i32, hardware_channels: i32) -> bool {
        self.enabled && Self::can_upmix(source_channels, hardware_channels)
    }

    /// Returns `true` when the output hardware layout includes overhead height channels
    /// (`> 8` channels, e.g. 5.1.4 / 7.1.2 / 7.1.4). Layouts with `3..=8` channels are Bed-Only.
    #[inline]
    pub fn has_height_channels(hardware_channels: i32) -> bool {
        hardware_channels > 8
    }

    /// Short speaker topology tag for compact HUD buttons (e.g. `"5.1"`, `"7.1"`, `"7.1.4"`).
    #[inline]
    pub fn short_tag(hardware_channels: i32) -> &'static str {
        if hardware_channels <= 2 {
            "2.0"
        } else if hardware_channels <= 4 {
            "4.0"
        } else if hardware_channels <= 6 {
            "5.1"
        } else if hardware_channels <= 8 {
            "7.1"
        } else if hardware_channels <= 10 {
            "7.1.2"
        } else {
            "7.1.4"
        }
    }

    /// Descriptive speaker topology label distinguishing Bed-Only vs. Bed + Height systems.
    #[inline]
    pub fn layout_badge(hardware_channels: i32) -> &'static str {
        if hardware_channels <= 2 {
            "Stereo/Mono Output — No Change"
        } else if hardware_channels <= 4 {
            "4.0 Quad — Bed Only"
        } else if hardware_channels <= 6 {
            "5.1 Surround — Bed Only"
        } else if hardware_channels <= 8 {
            "7.1 Surround — Bed Only"
        } else if hardware_channels <= 10 {
            "7.1.2 Surround — Bed + Height"
        } else {
            "7.1.4 Surround — Bed + Height"
        }
    }

    /// Evaluate the Linkwitz-Riley 2nd-order crossover routing magnitudes at `freq_hz`
    /// for either a Bed-Only (`has_height == false`) or Bed + Height (`has_height == true`) system.
    /// Returns `(routing_contour_gain, lfe_mag, side_rear_bed_mag, height_mag)`.
    pub fn evaluate_band_routing_for_layout(
        &self,
        freq_hz: f32,
        has_height: bool,
    ) -> (f32, f32, f32, f32) {
        let f = freq_hz.clamp(20.0, 20000.0);
        let eq_db = self.evaluate_eq_db(f);
        let eq_gain = 10.0f32.powf(eq_db / 20.0);

        let f_sub = self.sub_crossover_hz.clamp(40.0, 250.0);
        let sub_ratio_sq = (f / f_sub).powi(2);
        let lfe_lp = 1.0 / (1.0 + sub_ratio_sq);
        let lfe_mag = eq_gain * lfe_lp * self.lfe_level;

        if has_height {
            let f_height = self.height_crossover_hz.clamp(800.0, 10000.0);
            let height_ratio_sq = (f / f_height).powi(2);
            let height_lp = 1.0 / (1.0 + height_ratio_sq);
            let height_hp = height_ratio_sq / (1.0 + height_ratio_sq);

            let bed_mag = eq_gain
                * (height_lp + self.surround_high_bleed.clamp(0.0, 1.0) * height_hp)
                * self.side_rear_level;
            let height_mag = eq_gain * height_hp * self.height_level;
            (eq_gain, lfe_mag, bed_mag, height_mag)
        } else {
            // Bed-Only surround (4.0 / 5.1 / 7.1): full-bandwidth surround bed routing without height split
            let bed_mag = eq_gain * self.side_rear_level;
            (eq_gain, lfe_mag, bed_mag, 0.0)
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub song_title: String,
    pub artist: String,
    pub module_type: String,
    pub duration_seconds: f64,
    pub bitrate: Option<u32>,
    pub audio_tracks: Vec<AudioTrackInfo>,
    pub selected_audio_track: usize,
    pub active_audio_tracks: Vec<usize>,
    pub audio_track_volumes: Vec<f32>,
    pub multi_track_mix_mode: bool,
    pub audio_track_request: Option<usize>,
    pub audio_mix_request: Option<Vec<(usize, f32)>>,
    pub current_seconds: f64,
    pub seek_request: Option<f64>,
    pub scrub_target_seconds: Option<f64>,
    pub seek_epoch: u64,
    pub is_paused: bool,
    pub bpm: i32,
    pub speed: i32,
    pub num_channels: i32,
    pub hardware_channels: i32,
    pub source_channels: i32,
    pub raw_channel_vus: Vec<f32>,
    pub channel_vus: Vec<f32>,
    pub peak_vus: Vec<f32>,
    pub raw_spectrum_data: Vec<f32>,
    pub spectrum_data: Vec<f32>,
    pub spectrum_peaks: Vec<f32>,
    pub spectrum_history: VecDeque<Vec<f32>>,
    pub waveform_history: VecDeque<Vec<f32>>,
    pub waveform_history_push_count: u64,
    pub raw_waveform: Vec<f32>,
    pub raw_audio_channels: Vec<Vec<f32>>,
    pub gpu_spectrum_data: Vec<f32>,
    pub fire_heat: Vec<f32>,
    pub lookahead_timeline: Vec<f32>,
    pub lookahead_queue: VecDeque<(f64, [f32; 8])>,
    pub lookahead_sample_buffer: Vec<f32>,
    pub lookahead_buffer_start_time: f64,
    pub show_hud: bool,
    pub gpu_fft: bool,
    pub max_frequency: f32,
    pub num_samples: i32,
    pub num_instruments: i32,
    pub num_patterns: i32,
    pub current_tracker_order: i32,
    pub current_tracker_row: i32,
    pub tracker_row_history: VecDeque<(i32, i32)>,
    pub current_tracker_row_string: String,
    pub tracker_patterns_by_order: Vec<Vec<String>>,
    pub tracker_channels: Option<i32>,
    pub load_request: Option<String>,
    pub video_mode: u32,
    pub has_video_stream: bool,
    pub playlist: Vec<String>,
    pub playlist_index: usize,
    pub passthrough_enabled: bool,
    pub file_loaded: bool,
    pub video_info: Option<String>,
    pub show_stats: bool,
    pub show_help: bool,
    pub stats: PerformanceStats,
    pub current_fps: f32,
    pub current_sample_rate: f32,
    pub track_ended: bool,
    pub visualizer_mode: u32,
    pub visual_width: u32,
    pub target_fps: u32,
    pub current_visualizer_idx: usize,
    pub video_frame_rx: Option<crossbeam_channel::Receiver<VideoFrame>>,
    pub free_video_frame_tx: Option<crossbeam_channel::Sender<VideoFrame>>,
    pub is_file_picker_open: bool,
    pub open_file_request: bool,
    pub is_url_dialog_open: bool,
    pub focus_url_input: bool,
    pub url_input_text: String,
    pub url_history: Vec<(String, String)>,
    pub egui_gamepad_events: Vec<egui::Event>,
    pub force_stereo_downmix: bool,
    pub append_to_playlist: bool,
    pub panel_split_ratio: f32,
    pub gamepad_type: GamepadType,
    pub has_gamepad: bool,
    // Visualizer Picker & Hardware Capability / Runtime Frame-Budget Watchdog
    pub show_vis_picker: bool,
    pub vis_picker_cursor: usize,
    pub vis_picker_scroll_to_cursor: bool,
    pub vis_enabled: Vec<bool>,
    pub vis_user_overridden: Vec<bool>,
    pub gpu_capability_tier: GpuCapabilityTier,
    pub gpu_adapter_name: String,
    pub vis_perf_watch_idx: usize,
    pub vis_active_frames: u32,
    pub vis_low_fps_frames: u32,
    pub osd_text: Option<String>,
    pub osd_timer: f32,
    pub cumulative_scrub: f64,
    pub audio_device_lost: bool,
    pub biolum_top_down: bool,
    pub selected_audio_device: Option<String>,
    pub available_audio_devices: Vec<String>,
    pub audio_device_change_request: Option<String>,
    pub audio_topology_reload_request: Option<String>,
    pub active_audio_sink_id: Option<String>,
    pub active_audio_sink_description: Option<String>,
    pub active_audio_sink_channels: u16,
    pub is_dummy_audio_output: bool,
    pub passthrough_change_request: Option<bool>,
    pub show_tui_device_picker: bool,
    pub tui_device_picker_cursor: usize,
    pub lyrics: Option<std::sync::Arc<crate::lyrics::Lyrics>>,
    pub hovered_file: Option<String>,
    pub drag_cursor_pos: Option<[f32; 2]>,
    pub track_info_rect: Option<[f32; 4]>,
    pub mobile_hud_tab: MobileHudTab,
    pub surround_upmix: SurroundUpmixConfig,
}

pub fn get_history_file_path() -> std::path::PathBuf {
    if let Some(proj_dirs) = directories::ProjectDirs::from("com", "RustTracker", "RustTracker") {
        let dir = proj_dirs.data_dir();
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
        }
        dir.join("url_history.txt")
    } else {
        std::path::PathBuf::from("url_history.txt")
    }
}

impl AppState {
    pub fn new(title: String) -> Self {
        let is_mic = title == "Microphone Input";
        let default_audio_device = crate::audio::get_default_audio_device_name(is_mic);
        let available_audio_devices = crate::audio::get_available_audio_devices(is_mic);

        let mut history = VecDeque::new();
        for _ in 0..120 {
            history.push_back(vec![0.0; 1024]);
        }

        let mut wave_history = VecDeque::new();
        for _ in 0..60 {
            wave_history.push_back(vec![0.0; 1024]);
        }

        let is_steam_deck = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
            .map(|s| s.trim().to_lowercase().contains("valve"))
            .unwrap_or(false);

        let mut vis_enabled = vec![true; VISUALIZERS.len()];
        let vis_user_overridden = vec![false; VISUALIZERS.len()];

        // Solar Flare (ID 7): disabled on all systems due to computational cost and visual quality
        if let Some(idx) = VISUALIZERS.iter().position(|v| v.id == 7) {
            vis_enabled[idx] = false;
        }

        // Fire Simulation (ID 6): preserved for future development, disabled by default
        if let Some(idx) = VISUALIZERS.iter().position(|v| v.id == 6) {
            vis_enabled[idx] = false;
        }

        // 3D Glass Water Lyrics (ID 23): disabled by default (HUD lyrics remain active in the main panel)
        if let Some(idx) = VISUALIZERS.iter().position(|v| v.id == 23) {
            vis_enabled[idx] = false;
        }

        // Note: Heavy shader visualizers are no longer disabled based on target_os or DMI vendor.
        // Instead, `apply_gpu_capability_profile()` inspects the actual wgpu::AdapterInfo and
        // viewport pixel count upon Vulkan/GPU initialization, paired with `update_visualizer_perf_guard()`
        // at runtime.

        let mut url_history = Vec::new();
        if let Ok(data) = std::fs::read_to_string(get_history_file_path()) {
            for line in data.lines() {
                if let Some((url, title)) = line.split_once('|')
                    && !url_history.iter().any(|(u, _)| u == url)
                {
                    url_history.push((url.to_string(), title.to_string()));
                }
            }
        }

        AppState {
            passthrough_enabled: false,
            file_loaded: false,
            song_title: title,
            artist: "Unknown".to_string(),
            module_type: "Unknown".to_string(),
            duration_seconds: 0.0,
            bitrate: None,
            audio_tracks: Vec::new(),
            selected_audio_track: 0,
            active_audio_tracks: Vec::new(),
            audio_track_volumes: Vec::new(),
            multi_track_mix_mode: true,
            audio_track_request: None,
            audio_mix_request: None,
            current_seconds: 0.0,
            seek_request: None,
            scrub_target_seconds: None,
            seek_epoch: 0,
            is_paused: true, // Start paused to prevent audio playback before UI is ready
            bpm: 0,
            speed: 0,
            num_channels: 0,
            hardware_channels: 0,
            source_channels: 0,
            raw_channel_vus: Vec::new(),
            channel_vus: Vec::new(),
            peak_vus: Vec::new(),
            raw_spectrum_data: vec![0.0; 1024],
            spectrum_data: vec![0.0; 1024],
            spectrum_peaks: vec![0.0; 1024],
            spectrum_history: history,
            waveform_history: wave_history,
            waveform_history_push_count: 0,
            raw_waveform: vec![0.0; 1024],
            raw_audio_channels: Vec::new(),
            gpu_spectrum_data: vec![0.0; 32 * 1024 * 2],
            fire_heat: vec![0.0; 1024],
            lookahead_timeline: vec![0.0; 600 * 8],
            lookahead_queue: VecDeque::with_capacity(1200),
            lookahead_sample_buffer: Vec::with_capacity(4096),
            lookahead_buffer_start_time: 0.0,
            show_hud: true,
            gpu_fft: true,
            max_frequency: 10000.0,
            num_samples: 0,
            num_instruments: 0,
            num_patterns: 0,
            current_tracker_order: 0,
            current_tracker_row: 0,
            tracker_row_history: VecDeque::with_capacity(128),
            current_tracker_row_string: String::new(),
            tracker_patterns_by_order: Vec::new(),
            tracker_channels: None,
            load_request: None,

            video_info: None,
            show_stats: std::env::var("SHOW_STATS").is_ok(),
            show_help: std::env::var("SHOW_HELP").is_ok(),
            stats: PerformanceStats::default(),
            current_fps: 0.0,
            current_sample_rate: 44100.0,
            playlist: Vec::new(),
            playlist_index: 0,
            track_ended: false,
            visualizer_mode: VISUALIZERS[0].id,
            visual_width: 1024,
            target_fps: 144,
            current_visualizer_idx: 0,
            video_frame_rx: None,
            free_video_frame_tx: None,
            video_mode: 0,
            has_video_stream: false,
            is_file_picker_open: false,
            open_file_request: false,
            is_url_dialog_open: false,
            focus_url_input: false,
            url_input_text: String::new(),
            url_history,
            egui_gamepad_events: Vec::new(),
            force_stereo_downmix: false,
            append_to_playlist: false,
            panel_split_ratio: 0.5,
            gamepad_type: if is_steam_deck {
                GamepadType::SteamDeck
            } else {
                GamepadType::Xbox
            },
            has_gamepad: is_steam_deck,
            show_vis_picker: false,
            vis_picker_cursor: 0,
            vis_picker_scroll_to_cursor: false,
            vis_enabled,
            vis_user_overridden,
            gpu_capability_tier: GpuCapabilityTier::High,
            gpu_adapter_name: String::new(),
            vis_perf_watch_idx: 0,
            vis_active_frames: 0,
            vis_low_fps_frames: 0,
            osd_text: None,
            osd_timer: 0.0,
            cumulative_scrub: 0.0,
            audio_device_lost: false,
            biolum_top_down: true,
            selected_audio_device: default_audio_device,
            available_audio_devices,
            audio_device_change_request: None,
            audio_topology_reload_request: None,
            active_audio_sink_id: None,
            active_audio_sink_description: None,
            active_audio_sink_channels: 0,
            is_dummy_audio_output: false,
            passthrough_change_request: None,
            show_tui_device_picker: false,
            tui_device_picker_cursor: 0,
            lyrics: None,
            hovered_file: None,
            drag_cursor_pos: None,
            track_info_rect: None,
            mobile_hud_tab: MobileHudTab::default(),
            surround_upmix: SurroundUpmixConfig::default(),
        }
    }

    /// Create a lightweight clone for the render pass, skipping heavy audio data fields
    /// that are only consumed by engine.update() (which runs under the mutex lock).
    /// This saves ~1.3 MB of deep copies per frame vs a full .clone().
    pub fn render_snapshot(&self) -> Self {
        AppState {
            passthrough_enabled: self.passthrough_enabled,
            song_title: self.song_title.clone(),
            artist: self.artist.clone(),
            module_type: self.module_type.clone(),
            duration_seconds: self.duration_seconds,
            bitrate: self.bitrate,
            audio_tracks: self.audio_tracks.clone(),
            selected_audio_track: self.selected_audio_track,
            active_audio_tracks: self.active_audio_tracks.clone(),
            audio_track_volumes: self.audio_track_volumes.clone(),
            multi_track_mix_mode: self.multi_track_mix_mode,
            audio_track_request: None,
            audio_mix_request: None,
            current_seconds: self.current_seconds,
            seek_request: None,
            scrub_target_seconds: self.scrub_target_seconds,
            seek_epoch: self.seek_epoch,
            is_paused: self.is_paused,
            bpm: self.bpm,
            speed: self.speed,
            num_channels: self.num_channels,
            hardware_channels: self.hardware_channels,
            source_channels: self.source_channels,
            raw_channel_vus: Vec::new(), // Skipped in snapshot
            channel_vus: if self.file_loaded {
                self.channel_vus.clone()
            } else {
                Vec::new()
            },
            peak_vus: if self.file_loaded {
                self.peak_vus.clone()
            } else {
                Vec::new()
            },
            raw_spectrum_data: Vec::new(),
            spectrum_data: if self.file_loaded && self.surround_upmix.enabled {
                self.spectrum_data.clone()
            } else {
                Vec::new()
            },
            spectrum_peaks: Vec::new(), // Only used in TUI / engine.update()
            spectrum_history: VecDeque::new(), // Skipped in snapshot
            waveform_history: VecDeque::new(), // Skipped in snapshot
            waveform_history_push_count: 0,
            raw_waveform: Vec::new(),
            raw_audio_channels: Vec::new(),
            gpu_spectrum_data: Vec::new(),
            fire_heat: Vec::new(), // Only used in engine.update() (under lock)
            lookahead_timeline: Vec::new(), // Only used in engine.update() (under lock)
            lookahead_queue: VecDeque::new(), // Only used under lock
            lookahead_sample_buffer: Vec::new(),
            lookahead_buffer_start_time: 0.0,
            show_hud: self.show_hud,
            gpu_fft: self.gpu_fft,
            max_frequency: self.max_frequency,
            num_samples: self.num_samples,
            num_instruments: self.num_instruments,
            num_patterns: self.num_patterns,
            current_tracker_order: self.current_tracker_order,
            current_tracker_row: self.current_tracker_row,
            tracker_row_history: if self.file_loaded {
                self.tracker_row_history.clone()
            } else {
                VecDeque::new()
            },
            current_tracker_row_string: if self.file_loaded {
                self.current_tracker_row_string.clone()
            } else {
                String::new()
            },
            tracker_patterns_by_order: if self.file_loaded {
                self.tracker_patterns_by_order.clone()
            } else {
                Vec::new()
            },
            tracker_channels: self.tracker_channels,
            load_request: None,
            video_mode: self.video_mode,
            has_video_stream: self.has_video_stream,
            file_loaded: self.file_loaded,
            video_info: self.video_info.clone(),
            show_stats: self.show_stats,
            show_help: self.show_help,
            stats: self.stats.clone(),
            current_fps: self.current_fps,
            current_sample_rate: self.current_sample_rate,
            playlist: self.playlist.clone(),
            playlist_index: self.playlist_index,
            track_ended: self.track_ended,
            visualizer_mode: self.visualizer_mode,
            visual_width: self.visual_width,
            target_fps: self.target_fps,
            current_visualizer_idx: self.current_visualizer_idx,
            video_frame_rx: self.video_frame_rx.clone(),
            free_video_frame_tx: self.free_video_frame_tx.clone(),
            is_file_picker_open: self.is_file_picker_open,
            open_file_request: false,
            is_url_dialog_open: self.is_url_dialog_open,
            focus_url_input: self.focus_url_input,
            url_input_text: self.url_input_text.clone(),
            url_history: self.url_history.clone(),
            egui_gamepad_events: Vec::new(),
            force_stereo_downmix: self.force_stereo_downmix,
            append_to_playlist: self.append_to_playlist,
            vis_enabled: self.vis_enabled.clone(),
            vis_user_overridden: self.vis_user_overridden.clone(),
            gpu_capability_tier: self.gpu_capability_tier,
            gpu_adapter_name: self.gpu_adapter_name.clone(),
            vis_perf_watch_idx: self.vis_perf_watch_idx,
            vis_active_frames: self.vis_active_frames,
            vis_low_fps_frames: self.vis_low_fps_frames,
            gamepad_type: self.gamepad_type,
            has_gamepad: self.has_gamepad,
            panel_split_ratio: self.panel_split_ratio,
            show_vis_picker: self.show_vis_picker,
            vis_picker_cursor: self.vis_picker_cursor,
            vis_picker_scroll_to_cursor: self.vis_picker_scroll_to_cursor,
            osd_text: self.osd_text.clone(),
            osd_timer: self.osd_timer,
            cumulative_scrub: self.cumulative_scrub,
            audio_device_lost: self.audio_device_lost,
            biolum_top_down: self.biolum_top_down,
            selected_audio_device: self.selected_audio_device.clone(),
            available_audio_devices: self.available_audio_devices.clone(),
            audio_device_change_request: self.audio_device_change_request.clone(),
            audio_topology_reload_request: self.audio_topology_reload_request.clone(),
            active_audio_sink_id: self.active_audio_sink_id.clone(),
            active_audio_sink_description: self.active_audio_sink_description.clone(),
            active_audio_sink_channels: self.active_audio_sink_channels,
            is_dummy_audio_output: self.is_dummy_audio_output,
            passthrough_change_request: self.passthrough_change_request,
            show_tui_device_picker: self.show_tui_device_picker,
            tui_device_picker_cursor: self.tui_device_picker_cursor,
            lyrics: self.lyrics.clone(),
            hovered_file: self.hovered_file.clone(),
            drag_cursor_pos: self.drag_cursor_pos,
            track_info_rect: self.track_info_rect,
            mobile_hud_tab: self.mobile_hud_tab,
            surround_upmix: self.surround_upmix,
        }
    }

    /// Returns true if a media track is currently active/playing (not on the splash screen,
    /// not microphone-only mode, and playback has not ended).
    pub fn has_active_playback(&self) -> bool {
        self.file_loaded
            && !self.track_ended
            && (!self.playlist.is_empty()
                || (!self.song_title.is_empty() && self.song_title != "Microphone Input"))
    }

    /// Returns true if the given cursor position (or `self.drag_cursor_pos`) lies within
    /// the last rendered Track Info section rectangle.
    pub fn is_over_track_info(&self, cursor_pos: Option<[f32; 2]>) -> bool {
        if let (Some(c_pos), Some(ti_rect)) =
            (cursor_pos.or(self.drag_cursor_pos), self.track_info_rect)
        {
            c_pos[0] >= ti_rect[0]
                && c_pos[0] <= ti_rect[2]
                && c_pos[1] >= ti_rect[1]
                && c_pos[1] <= ti_rect[3]
        } else {
            false
        }
    }

    /// Handles media file(s) dropped onto the window:
    /// - When dropped onto the Track Info section while a track is currently playing,
    ///   creates/updates the playlist with the dropped track(s) queued next (immediately
    ///   after the current track) without interrupting current playback.
    /// - When dropped anywhere else, or when nothing is playing, replaces the playlist
    ///   and plays the dropped file immediately as normal.
    pub fn handle_dropped_files(&mut self, expanded: Vec<String>, cursor_pos: Option<[f32; 2]>) {
        self.hovered_file = None;
        self.drag_cursor_pos = None;
        if expanded.is_empty() {
            return;
        }

        let is_over_ti = self.is_over_track_info(cursor_pos);
        if self.has_active_playback() && is_over_ti {
            if self.playlist.is_empty() && !self.song_title.is_empty() {
                self.playlist.push(self.song_title.clone());
                self.playlist_index = 0;
            }
            let insert_idx = (self.playlist_index + 1).min(self.playlist.len());
            let count = expanded.len();
            let file_name = std::path::Path::new(&expanded[0])
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            self.playlist.splice(insert_idx..insert_idx, expanded);
            if count == 1 {
                self.osd_text = Some(format!("Added to Playlist: {}", file_name));
            } else {
                self.osd_text = Some(format!("Added {} tracks to Playlist", count));
            }
            self.osd_timer = 3.0;
        } else {
            self.playlist = expanded;
            self.playlist_index = 0;
            self.load_request = Some(self.playlist[0].clone());
            self.file_loaded = true;
            self.is_paused = false;
            self.track_ended = false;
        }
    }

    /// Applies the hardware-based GPU capability tier once the Vulkan/Metal/DX12 adapter
    /// and viewport dimensions are known.
    ///
    /// On `GpuCapabilityTier::High` (discrete GPUs and flagship mobile/integrated GPUs such as
    /// Snapdragon 8 Gen 2/3/Elite Adreno 740/750/8xx, Immortalis-G720/G925, Apple Silicon,
    /// Radeon 680M/780M/890M, Intel Arc), all production visualizers remain enabled in rotation.
    /// On `GpuCapabilityTier::Constrained` (fillrate/power-constrained GPUs such as Mali-G710
    /// driving a 2560x1600 QHD+ panel, older integrated GPUs, or Steam Deck's 15W APU),
    /// visualizers marked `heavy_shader: true` start unchecked in auto-rotation unless the
    /// user explicitly selected or enabled them.
    pub fn apply_gpu_capability_profile(
        &mut self,
        info: &wgpu::AdapterInfo,
        width: u32,
        height: u32,
    ) {
        let viewport_pixels = width.saturating_mul(height);
        let tier = classify_gpu_capability(info, viewport_pixels);
        self.gpu_capability_tier = tier;
        self.gpu_adapter_name = info.name.clone();

        if tier == GpuCapabilityTier::Constrained {
            for (idx, v) in VISUALIZERS.iter().enumerate() {
                if v.heavy_shader && !self.vis_user_overridden[idx] {
                    self.vis_enabled[idx] = false;
                }
            }
            if !self.vis_enabled[self.current_visualizer_idx]
                && !self.vis_user_overridden[self.current_visualizer_idx]
                && let Some(first_enabled) = self.vis_enabled.iter().position(|&e| e)
            {
                self.current_visualizer_idx = first_enabled;
                self.visualizer_mode = VISUALIZERS[first_enabled].id;
            }
        }
    }

    /// Runtime frame-budget watchdog: monitors frame times while a heavy shader visualizer is
    /// actively rendering. If a non-user-overridden `heavy_shader` visualizer cannot sustain
    /// at least 85% of the target display refresh rate (e.g., < 51 FPS on a 60Hz panel) over a
    /// sustained window after warm-up, it is automatically unchecked from auto-rotation and
    /// advances to the next enabled visualizer.
    pub fn update_visualizer_perf_guard(&mut self, dt: f32, target_refresh_hz: f32) {
        let idx = self.current_visualizer_idx.min(VISUALIZERS.len() - 1);
        if self.vis_perf_watch_idx != idx {
            self.vis_perf_watch_idx = idx;
            self.vis_active_frames = 0;
            self.vis_low_fps_frames = 0;
        }

        // Only evaluate during active visualizer playback (not on Welcome screen, paused, or fullscreen video)
        if !self.file_loaded || self.is_paused || self.track_ended || self.video_mode == 3 {
            return;
        }

        // Respect explicit user selection or manual enable in the Visualizer Picker
        if self.vis_user_overridden[idx] || !VISUALIZERS[idx].heavy_shader {
            return;
        }

        self.vis_active_frames = self.vis_active_frames.saturating_add(1);
        // Allow 45 frames (~0.75s) for pipeline/shader warm-up after switching visualizers
        if self.vis_active_frames <= 45 {
            return;
        }

        let effective_hz = target_refresh_hz.clamp(30.0, 240.0);
        // 85% of target refresh rate (e.g. > 19.6ms frame time on a 60Hz display = < 51 FPS)
        let budget_dt = 1.0 / (effective_hz * 0.85);
        if dt > budget_dt && dt < 0.5 {
            self.vis_low_fps_frames = self.vis_low_fps_frames.saturating_add(1);
        } else {
            self.vis_low_fps_frames = self.vis_low_fps_frames.saturating_sub(1);
        }

        if self.vis_low_fps_frames >= 90 {
            let dropped_name = VISUALIZERS[idx].name;
            self.vis_enabled[idx] = false;
            self.vis_low_fps_frames = 0;
            self.vis_active_frames = 0;

            let len = VISUALIZERS.len();
            for offset in 1..=len {
                let next = (idx + offset) % len;
                if self.vis_enabled[next] {
                    self.current_visualizer_idx = next;
                    self.visualizer_mode = VISUALIZERS[next].id;
                    self.osd_text = Some(format!(
                        "Auto-disabled {} (< {:.0} FPS) -> {}",
                        dropped_name,
                        effective_hz * 0.85,
                        VISUALIZERS[next].name
                    ));
                    self.osd_timer = 3.0;
                    break;
                }
            }
        }
    }

    /// Enables or disables automatic surround upmixing with configurable crossover routing.
    /// Only affects playback when connected to a multi-channel surround output device (`hardware_channels > 2`).
    pub fn set_surround_upmix(&mut self, enabled: bool) {
        if enabled
            && self.file_loaded
            && self.hardware_channels > 2
            && self.source_channels >= self.hardware_channels
        {
            self.surround_upmix.enabled = false;
            let short_tag = SurroundUpmixConfig::short_tag(self.hardware_channels);
            self.osd_text = Some(format!(
                "🔊 Native Discrete {} Audio ({}ch) — Upmix Not Applicable",
                short_tag, self.source_channels
            ));
            self.osd_timer = 2.5;
            return;
        }
        let prev_enabled = self.surround_upmix.enabled;
        self.surround_upmix.enabled = enabled;
        self.surround_upmix.user_overridden = true;
        if enabled {
            self.force_stereo_downmix = false;
            if self.passthrough_enabled || self.stats.bitstream_active {
                self.passthrough_enabled = false;
            }
            if self.file_loaded && (self.stats.bitstream_active || self.hardware_channels <= 2) {
                self.passthrough_change_request = Some(false);
            }
            if self.hardware_channels > 2 {
                self.num_channels = self.hardware_channels.min(12);
                let ch_count = self.num_channels as usize;
                self.channel_vus.resize(ch_count, 0.0);
                self.peak_vus.resize(ch_count, 0.0);
                if self.file_loaded {
                    self.show_hud = true;
                    if self.video_mode == 3 {
                        self.video_mode = 0;
                    }
                }
                let badge = SurroundUpmixConfig::layout_badge(self.hardware_channels);
                self.osd_text = Some(format!("🔊 Surround Upmix: ON ({})", badge));
            } else {
                self.osd_text =
                    Some("🔊 Surround Upmix: Armed (Stereo/Mono Output — No Change)".to_string());
            }
        } else {
            if self.file_loaded
                && prev_enabled
                && self.passthrough_enabled
                && self.hardware_channels > 2
            {
                self.passthrough_change_request = Some(true);
            }
            if self.hardware_channels > 2 {
                let upmix_ch = self.hardware_channels.min(12) as usize;
                self.num_channels = if let Some(tc) = self.tracker_channels {
                    tc + 2
                } else if self.source_channels > 0 {
                    self.source_channels
                } else if !self.raw_channel_vus.is_empty() && self.raw_channel_vus.len() != upmix_ch
                {
                    self.raw_channel_vus.len() as i32
                } else {
                    2
                };
                let ch_count = self.num_channels as usize;
                self.channel_vus.resize(ch_count, 0.0);
                self.peak_vus.resize(ch_count, 0.0);
            }
            self.osd_text = Some("🔊 Surround Upmix: OFF".to_string());
        }
        self.osd_timer = 2.5;
    }

    /// Toggles automatic surround upmixing with configurable crossover routing.
    pub fn toggle_surround_upmix(&mut self) {
        let next = !self.surround_upmix.enabled;
        self.set_surround_upmix(next);
    }

    /// Updates the OSD for a relative seek jump (`delta` in seconds).
    /// Repeated jumps in the same direction while the seek OSD is still active accumulate
    /// the total time jumped, resetting once the OSD timer expires (after 2.0 seconds)
    /// or when jumping in the opposite direction.
    pub fn record_seek_osd(&mut self, delta: f64) {
        if delta.abs() < 0.1 {
            self.cumulative_scrub = 0.0;
            self.osd_text = Some(format!("{}1 Frame", if delta > 0.0 { "+" } else { "-" }));
            self.osd_timer = 2.0;
            return;
        }

        let is_active_seek_osd = self.osd_timer > 0.0
            && self
                .osd_text
                .as_deref()
                .is_some_and(|s| (s.starts_with('+') || s.starts_with('-')) && s.ends_with('s'));

        if is_active_seek_osd && self.cumulative_scrub * delta > 0.0 {
            self.cumulative_scrub += delta;
        } else {
            self.cumulative_scrub = delta;
        }

        let sign = if self.cumulative_scrub >= 0.0 {
            "+"
        } else {
            ""
        };
        self.osd_text = Some(format!("{}{:.1}s", sign, self.cumulative_scrub));
        self.osd_timer = 2.0;
    }

    /// Advances the OSD countdown timer by `dt` seconds and clears the OSD text
    /// and cumulative seek state once it expires.
    pub fn tick_osd_timer(&mut self, dt: f32) {
        if self.osd_timer > 0.0 {
            self.osd_timer = (self.osd_timer - dt).max(0.0);
            if self.osd_timer == 0.0 {
                self.osd_text = None;
                self.cumulative_scrub = 0.0;
            }
        }
    }

    /// Enables or disables 12-channel 7.1.4 (11.1, CICP Layout 19) MPEG-H 3D Audio IEC 61937-13
    /// HDMI bitstreaming for surround upmixing and native >8ch playback on HDMI AVRs.
    pub fn set_mpegh_bitstream(&mut self, enabled: bool) {
        self.surround_upmix.mpegh_bitstream = enabled;
        self.surround_upmix.mpegh_user_overridden = true;
        if enabled {
            self.force_stereo_downmix = false;
            if self.passthrough_enabled {
                self.passthrough_enabled = false;
            }
            self.osd_text = Some("🔊 11.1 (7.1.4 Spatial Audio / MPEG-H 3D): ON".to_string());
        } else {
            self.osd_text = Some("🔊 11.1 (7.1.4): OFF (Uncompressed 7.1 LPCM)".to_string());
        }
        self.osd_timer = 3.0;
        if self.file_loaded {
            self.passthrough_change_request = Some(self.passthrough_enabled);
        }
    }

    /// Toggles 12-channel 7.1.4 (11.1, CICP Layout 19) MPEG-H 3D Audio IEC 61937-13 HDMI bitstreaming.
    pub fn toggle_mpegh_bitstream(&mut self) {
        let next = !self.surround_upmix.mpegh_bitstream;
        self.set_mpegh_bitstream(next);
    }
}

/// Format a playback timestamp or duration in seconds as `H:MM:SS` (when >= 1 hour
/// or `force_hours` is true) or `MM:SS` (when < 1 hour).
pub fn format_timeline_time(secs: f64, force_hours: bool) -> String {
    let total_secs = if !secs.is_finite() || secs <= 0.0 {
        0
    } else {
        secs.floor() as u64
    };
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let rem_secs = total_secs % 60;
    if hours > 0 || force_hours {
        format!("{}:{:02}:{:02}", hours, mins, rem_secs)
    } else {
        format!("{:02}:{:02}", mins, rem_secs)
    }
}
