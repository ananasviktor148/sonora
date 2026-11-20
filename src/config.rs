use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

static DATA_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

pub const EQ_FREQUENCIES: [f64; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

pub const EQ_LABELS: [&str; 10] = [
    "31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];

pub const STANDARD_SAMPLE_RATES: [(u32, &str); 10] = [
    (44_100, "44.1 kHz"),
    (48_000, "48 kHz"),
    (88_200, "88.2 kHz"),
    (96_000, "96 kHz"),
    (176_400, "176.4 kHz"),
    (192_000, "192 kHz"),
    (352_800, "352.8 kHz"),
    (384_000, "384 kHz"),
    (705_600, "705.6 kHz"),
    (768_000, "768 kHz"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpsampleMode {
    Bypass,
    Factor2x,
    Factor4x,
    Factor8x,
    Fixed(u32),
}

impl UpsampleMode {
    pub fn label(&self) -> String {
        match self {
            Self::Bypass => "Bypass (Bit-Perfect)".to_string(),
            Self::Factor2x => "2x Integer Family".to_string(),
            Self::Factor4x => "4x Integer Family".to_string(),
            Self::Factor8x => "8x Integer Family".to_string(),
            Self::Fixed(r) => format!("Fixed {}", format_rate(*r)),
        }
    }

    /// Compute the target sample rate given an input sample rate, clamped to 768 kHz.
    pub fn resolve_rate(&self, input_rate: u32) -> u32 {
        if input_rate == 0 {
            return 48_000;
        }
        match self {
            Self::Bypass => input_rate,
            Self::Factor2x => (input_rate.saturating_mul(2)).min(768_000),
            Self::Factor4x => (input_rate.saturating_mul(4)).min(768_000),
            Self::Factor8x => (input_rate.saturating_mul(8)).min(768_000),
            Self::Fixed(rate) => *rate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResamplerQuality {
    Fast,
    High,
    Audiophile,
    Reference,
}

impl ResamplerQuality {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fast => "Fast (64-tap Linear)",
            Self::High => "High (128-tap Blackman)",
            Self::Audiophile => "Audiophile (256-tap BH2 Cubic)",
            Self::Reference => "Reference (512-tap BH2 Cubic)",
        }
    }

    pub fn sinc_len(&self) -> usize {
        match self {
            Self::Fast => 64,
            Self::High => 128,
            Self::Audiophile => 256,
            Self::Reference => 512,
        }
    }

    pub fn oversampling_factor(&self) -> usize {
        match self {
            Self::Fast => 64,
            Self::High => 128,
            Self::Audiophile => 256,
            Self::Reference => 512,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterRolloff {
    Sharp,
    Balanced,
    Gentle,
}

impl FilterRolloff {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sharp => "Sharp (0.95 Nyquist)",
            Self::Balanced => "Balanced (0.91 Nyquist)",
            Self::Gentle => "Gentle / Slow (0.85 Nyquist)",
        }
    }

    pub fn f_cutoff(&self) -> f32 {
        match self {
            Self::Sharp => 0.95,
            Self::Balanced => 0.91,
            Self::Gentle => 0.85,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DsdOutputMode {
    Pcm88k,
    Pcm176k,
    Pcm352k,
    DoP,
}

impl DsdOutputMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pcm88k => "PCM 88.2 kHz (32-bit float)",
            Self::Pcm176k => "PCM 176.4 kHz (Audiophile)",
            Self::Pcm352k => "PCM 352.8 kHz (DXD Studio)",
            Self::DoP => "DoP v1.1 (DSD over PCM)",
        }
    }

    pub fn target_pcm_rate(&self) -> Option<u32> {
        match self {
            Self::Pcm88k => Some(88_200),
            Self::Pcm176k => Some(176_400),
            Self::Pcm352k => Some(352_800),
            Self::DoP => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DsdSettings {
    pub output_mode: DsdOutputMode,
    pub cutoff_hz: u32,
    pub gain_boost_6db: bool,
}

impl Default for DsdSettings {
    fn default() -> Self {
        Self {
            output_mode: DsdOutputMode::Pcm176k,
            cutoff_hz: 30_000,
            gain_boost_6db: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownmixMode {
    Auto,
    NativePassthrough,
    StereoItu,
    HeadphoneSpatial,
}

impl DownmixMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Auto => "Auto (Native / ITU Downmix)",
            Self::NativePassthrough => "Native Multi-Channel",
            Self::StereoItu => "Stereo Downmix (ITU-R BS.775)",
            Self::HeadphoneSpatial => "Headphone Spatial Crossfeed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiChannelSettings {
    pub downmix_mode: DownmixMode,
    pub include_lfe_in_downmix: bool,
    pub lfe_gain_db: f32,
    pub center_gain_db: f32,
    pub surround_gain_db: f32,
    pub crossfeed_amount: f32,
}

impl Default for MultiChannelSettings {
    fn default() -> Self {
        Self {
            downmix_mode: DownmixMode::Auto,
            include_lfe_in_downmix: true,
            lfe_gain_db: -6.0,
            center_gain_db: -3.0,
            surround_gain_db: -3.0,
            crossfeed_amount: 0.25,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
    pub preset_name: String,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            bands_db: [0.0; 10],
            preset_name: "Flat".to_string(),
        }
    }
}

impl EqSettings {
    pub fn presets() -> &'static [(&'static str, f32, [f32; 10])] {
        &[
            ("Flat", 0.0, [0.0; 10]),
            (
                "Audiophile Warm",
                -1.0,
                [1.5, 2.0, 1.5, 0.5, 0.0, -0.5, 0.0, 1.0, 1.5, 1.0],
            ),
            (
                "Studio Reference",
                0.0,
                [0.5, 0.0, 0.0, -0.5, 0.0, 0.0, 0.5, 0.5, 1.0, 0.5],
            ),
            (
                "Vocal Clarity",
                -1.5,
                [-1.0, -0.5, 0.0, 1.5, 3.0, 3.5, 2.5, 1.0, 0.5, 0.0],
            ),
            (
                "Deep Bass",
                -3.0,
                [5.0, 4.5, 3.0, 1.0, 0.0, 0.0, 0.5, 1.0, 1.0, 0.5],
            ),
            (
                "Classical Hall",
                -1.0,
                [1.0, 0.5, 0.0, 0.0, -0.5, -0.5, 0.5, 1.5, 2.5, 2.0],
            ),
            (
                "Jazz Club",
                -1.0,
                [2.0, 1.5, 1.0, 1.5, -0.5, -0.5, 0.5, 1.5, 2.0, 1.5],
            ),
            (
                "Electronic",
                -2.5,
                [4.0, 3.5, 1.5, 0.0, -1.0, 0.5, 1.5, 2.5, 3.5, 3.0],
            ),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoVolumeMode {
    Off,
    Song,
    Album,
}

impl AutoVolumeMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "Off (Original Master Level)",
            Self::Song => "Song (Normalize Each Track)",
            Self::Album => "Album (Preserve Album Dynamics)",
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Song => "SONG",
            Self::Album => "ALBUM",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoVolumeSettings {
    pub mode: AutoVolumeMode,
    /// Target integrated loudness in LUFS (e.g. -16.0 audiophile, -18.0 ReplayGain, -14.0 streaming, -23.0 EBU R128).
    pub target_lufs: f32,
    /// Prevent clipping by limiting positive boost against peak level (-1.0 dBFS ceiling).
    pub prevent_clipping: bool,
    /// Prefer embedded REPLAYGAIN / R128 tags when present before falling back to EBU R128 scan.
    pub prefer_replaygain_tags: bool,
}

impl Default for AutoVolumeSettings {
    fn default() -> Self {
        Self {
            mode: AutoVolumeMode::Off,
            target_lufs: -16.0,
            prevent_clipping: true,
            prefer_replaygain_tags: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub volume: f32,
    pub headroom_db: f32,
    pub soft_clip: bool,
    pub selected_device: Option<String>,
    pub upsample_mode: UpsampleMode,
    pub resampler_quality: ResamplerQuality,
    pub filter_rolloff: FilterRolloff,
    pub dsd: DsdSettings,
    pub multichannel: MultiChannelSettings,
    pub eq: EqSettings,
    pub auto_volume: AutoVolumeSettings,
    pub lyrics_font_size: f32,
    pub lyrics_auto_scroll: bool,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            volume: 0.80,
            headroom_db: -1.0,
            soft_clip: true,
            selected_device: None,
            upsample_mode: UpsampleMode::Bypass,
            resampler_quality: ResamplerQuality::Audiophile,
            filter_rolloff: FilterRolloff::Sharp,
            dsd: DsdSettings::default(),
            multichannel: MultiChannelSettings::default(),
            eq: EqSettings::default(),
            auto_volume: AutoVolumeSettings::default(),
            lyrics_font_size: 16.0,
            lyrics_auto_scroll: true,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
        }
    }
}

impl AppConfig {
    /// Override the storage directory (used on Android to store files under `Context.filesDir`).
    #[allow(dead_code)]
    pub fn set_data_dir_override(dir: PathBuf) {
        let _ = DATA_DIR_OVERRIDE.set(dir);
    }

    pub fn data_dir() -> PathBuf {
        if let Some(dir) = DATA_DIR_OVERRIDE.get() {
            return dir.clone();
        }

        #[cfg(target_os = "windows")]
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        #[cfg(not(target_os = "windows"))]
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));

        base.join("HiResAudioPlayer")
    }

    pub fn config_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    pub fn lyrics_store_dir() -> PathBuf {
        Self::data_dir().join("lyrics")
    }

    pub fn playlists_path() -> PathBuf {
        Self::data_dir().join("playlists.json")
    }

    pub fn loudness_cache_path() -> PathBuf {
        Self::data_dir().join("loudness_cache.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Self>(&content) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

pub fn format_rate(rate: u32) -> String {
    if rate >= 1_000_000 {
        format!("{:.4} MHz", rate as f64 / 1_000_000.0)
    } else if rate >= 1_000 {
        let khz = rate as f64 / 1_000.0;
        if (khz - khz.round()).abs() < 0.01 {
            format!("{} kHz", khz.round() as u32)
        } else {
            format!("{:.1} kHz", khz)
        }
    } else {
        format!("{} Hz", rate)
    }
}
