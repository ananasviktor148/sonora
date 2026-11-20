use std::collections::HashMap;
use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use anyhow::{anyhow, Result};
use lofty::prelude::*;
use lofty::tag::ItemKey;
use serde::{Deserialize, Serialize};

use crate::audio::decoder::open_decoder;
use crate::audio::demo::demo_spec_for_path;
use crate::config::{AppConfig, AutoVolumeMode, AutoVolumeSettings, DsdOutputMode, DsdSettings};

const REPLAYGAIN_REF_LUFS: f32 = -18.0;
const R128_REF_LUFS: f32 = -23.0;
const PEAK_CEILING_DBFS: f32 = -1.0;
const MAX_ALBUM_PEERS_TO_SCAN: usize = 40;

// ── Public Data Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackLoudness {
    pub integrated_lufs: f32,
    pub peak_linear: f32,
    pub is_dsd: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ReplayGainTags {
    pub track_gain_db: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain_db: Option<f32>,
    pub album_peak: Option<f32>,
    pub r128_track_gain_db: Option<f32>,
    pub r128_album_gain_db: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct AutoVolumeResult {
    pub mode: AutoVolumeMode,
    pub gain_db: f32,
    pub measured_lufs: f32,
    pub peak_dbfs: Option<f32>,
    pub peak_limited: bool,
    pub source_label: String,
}

// ── ITU-R BS.1770-4 / EBU R128 K-Weighting Analyzer ──────────────────────────

#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// Stage 1: ITU-R BS.1770-4 acoustic head pre-filter (+4 dB high-shelf above ~1.68 kHz).
    fn k_weighting_stage1(sample_rate: f64) -> Self {
        let f0 = 1681.974450955533_f64.min(sample_rate * 0.45);
        let g_db = 3.999843853973347_f64;
        let q = 0.7071752369554196_f64;

        let k = (PI * f0 / sample_rate).tan();
        let vh = 10.0_f64.powf(g_db / 20.0);
        let vb = vh.powf(0.4996667741545416);

        let a0 = 1.0 + k / q + k * k;
        let b0 = (vh + vb * k / q + k * k) / a0;
        let b1 = 2.0 * (k * k - vh) / a0;
        let b2 = (vh - vb * k / q + k * k) / a0;
        let a1 = 2.0 * (k * k - 1.0) / a0;
        let a2 = (1.0 - k / q + k * k) / a0;

        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Stage 2: ITU-R BS.1770-4 Revised Low-Frequency B-weighting (RLB) high-pass (~38.1 Hz).
    fn k_weighting_stage2(sample_rate: f64) -> Self {
        let f0 = 38.13547087602444_f64.min(sample_rate * 0.45);
        let q = 0.5003270373238773_f64;

        let k = (PI * f0 / sample_rate).tan();
        let a0 = 1.0 + k / q + k * k;
        let b0 = 1.0 / a0;
        let b1 = -2.0 / a0;
        let b2 = 1.0 / a0;
        let a1 = 2.0 * (k * k - 1.0) / a0;
        let a2 = (1.0 - k / q + k * k) / a0;

        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// ITU-R BS.1770-4 channel weightings (`G_i`) matching standard PCM/SMPTE layouts.
fn bs1770_channel_weights(channels: usize) -> Vec<f64> {
    match channels {
        0 => Vec::new(),
        1 => vec![1.0],
        2 => vec![1.0, 1.0],
        3 => vec![1.0, 1.0, 1.0],
        4 => vec![1.0, 1.0, 1.41, 1.41],
        5 => vec![1.0, 1.0, 1.0, 1.41, 1.41],
        6 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41], // FL, FR, FC, LFE, SL, SR
        7 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 1.41], // FL, FR, FC, LFE, BC, SL, SR
        8 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 1.41, 1.41], // 7.1
        n => {
            let mut w = vec![1.0; n];
            if n > 3 {
                w[3] = 0.0; // LFE excluded per BS.1770-4
            }
            for item in w.iter_mut().skip(4) {
                *item = 1.41;
            }
            w
        }
    }
}

/// Streaming ITU-R BS.1770-4 / EBU R128 integrated loudness and sample-peak meter.
pub struct LoudnessMeter {
    channels: usize,
    weights: Vec<f64>,
    stage1: Vec<Biquad>,
    stage2: Vec<Biquad>,
    subblock_frames: usize,
    sub_acc_energy: f64,
    sub_acc_frames: usize,
    subblocks: Vec<f64>,
    peak_linear: f32,
}

impl LoudnessMeter {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let sr = sample_rate.max(8_000) as f64;
        let ch = channels.max(1);
        let subblock_frames = ((sr * 0.10).round() as usize).max(1); // 100 ms hop (75% overlap of 400 ms blocks)
        Self {
            channels: ch,
            weights: bs1770_channel_weights(ch),
            stage1: vec![Biquad::k_weighting_stage1(sr); ch],
            stage2: vec![Biquad::k_weighting_stage2(sr); ch],
            subblock_frames,
            sub_acc_energy: 0.0,
            sub_acc_frames: 0,
            subblocks: Vec::with_capacity(512),
            peak_linear: 0.0,
        }
    }

    /// Reset K-weighting filter state when jumping to a non-contiguous seek window.
    pub fn reset_filter_state(&mut self) {
        for f in &mut self.stage1 {
            f.reset();
        }
        for f in &mut self.stage2 {
            f.reset();
        }
        self.sub_acc_energy = 0.0;
        self.sub_acc_frames = 0;
    }

    /// Feed a planar chunk (`channels × frames`).
    #[allow(clippy::needless_range_loop)]
    pub fn push_planar(&mut self, planar: &[Vec<f32>]) {
        if planar.is_empty() {
            return;
        }
        let frames = planar[0].len();
        let active_ch = planar.len().min(self.channels);
        if frames == 0 || active_ch == 0 {
            return;
        }

        for f in 0..frames {
            let mut weighted_sq = 0.0_f64;
            for ch in 0..active_ch {
                let s = planar[ch][f];
                let a = s.abs();
                if a > self.peak_linear {
                    self.peak_linear = a;
                }
                let w = self.weights[ch];
                if w > 0.0 {
                    let y1 = self.stage1[ch].process(s as f64);
                    let y2 = self.stage2[ch].process(y1);
                    weighted_sq += w * y2 * y2;
                }
            }
            self.sub_acc_energy += weighted_sq;
            self.sub_acc_frames += 1;

            if self.sub_acc_frames >= self.subblock_frames {
                self.subblocks
                    .push(self.sub_acc_energy / self.sub_acc_frames as f64);
                self.sub_acc_energy = 0.0;
                self.sub_acc_frames = 0;
            }
        }
    }

    /// Compute ITU-R BS.1770-4 gated integrated loudness (LUFS) and linear sample peak.
    pub fn finalize(&self) -> (f32, f32) {
        let mut blocks: Vec<f64> = Vec::new();
        if self.subblocks.len() >= 4 {
            blocks.reserve(self.subblocks.len() - 3);
            for w in self.subblocks.windows(4) {
                blocks.push(0.25 * (w[0] + w[1] + w[2] + w[3]));
            }
        } else if !self.subblocks.is_empty() {
            let mean: f64 = self.subblocks.iter().sum::<f64>() / self.subblocks.len() as f64;
            blocks.push(mean);
        } else if self.sub_acc_frames > 0 {
            blocks.push(self.sub_acc_energy / self.sub_acc_frames as f64);
        }

        if blocks.is_empty() {
            return (-70.0, self.peak_linear);
        }

        // Absolute gate: -70.0 LKFS
        let abs_thresh = 10.0_f64.powf((-70.0 + 0.691) / 10.0);
        let abs_gated: Vec<f64> = blocks.into_iter().filter(|&z| z > abs_thresh).collect();
        if abs_gated.is_empty() {
            return (-70.0, self.peak_linear);
        }

        // Relative gate: -10.0 dB below ungated mean energy
        let abs_mean = abs_gated.iter().sum::<f64>() / abs_gated.len() as f64;
        let rel_thresh = abs_mean * 0.1; // 10^(-10/10) = 0.1
        let mut rel_sum = 0.0_f64;
        let mut rel_count = 0usize;
        for z in abs_gated {
            if z > rel_thresh {
                rel_sum += z;
                rel_count += 1;
            }
        }

        let final_energy = if rel_count > 0 {
            rel_sum / rel_count as f64
        } else {
            abs_mean
        };

        let lufs = if final_energy > 1e-12 {
            (-0.691 + 10.0 * final_energy.log10()) as f32
        } else {
            -70.0
        };
        (lufs, self.peak_linear)
    }
}

// ── Embedded ReplayGain & R128 Tag Reader ────────────────────────────────────

fn canonical_tag_key(key: &ItemKey) -> String {
    let raw = match key {
        ItemKey::Unknown(s) => s.as_str().to_string(),
        other => format!("{:?}", other),
    };
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

pub fn parse_db_tag_value(raw: &str) -> Option<f32> {
    let cleaned = raw
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_ascii_whitespace())
        .trim()
        .replace(',', ".");
    let val: f32 = cleaned.parse().ok()?;
    if val.is_finite() {
        Some(val)
    } else {
        None
    }
}

pub fn parse_r128_q78_db(raw: &str) -> Option<f32> {
    let v: f32 = raw.trim().parse().ok()?;
    if v.is_finite() {
        Some(v / 256.0)
    } else {
        None
    }
}

pub fn read_replaygain_tags(path: &Path) -> ReplayGainTags {
    let mut tags = ReplayGainTags::default();
    if demo_spec_for_path(path).is_some() || !path.exists() {
        return tags;
    }

    let Ok(tagged) = lofty::read_from_path(path) else {
        return tags;
    };

    for tag in tagged.tags() {
        for item in tag.items() {
            let Some(val) = item.value().text() else {
                continue;
            };
            let key = canonical_tag_key(item.key());
            match key.as_str() {
                "REPLAYGAINTRACKGAIN" if tags.track_gain_db.is_none() => {
                    tags.track_gain_db = parse_db_tag_value(val);
                }
                "REPLAYGAINALBUMGAIN" if tags.album_gain_db.is_none() => {
                    tags.album_gain_db = parse_db_tag_value(val);
                }
                "REPLAYGAINTRACKPEAK" if tags.track_peak.is_none() => {
                    tags.track_peak = parse_db_tag_value(val).filter(|&p| p > 0.0);
                }
                "REPLAYGAINALBUMPEAK" if tags.album_peak.is_none() => {
                    tags.album_peak = parse_db_tag_value(val).filter(|&p| p > 0.0);
                }
                "R128TRACKGAIN" if tags.r128_track_gain_db.is_none() => {
                    tags.r128_track_gain_db = parse_r128_q78_db(val);
                }
                "R128ALBUMGAIN" if tags.r128_album_gain_db.is_none() => {
                    tags.r128_album_gain_db = parse_r128_q78_db(val);
                }
                _ => {}
            }
        }
    }

    tags
}

// ── Persistent Disk Loudness Cache ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedLoudnessEntry {
    fingerprint: String,
    loudness: TrackLoudness,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LoudnessCache {
    entries: HashMap<String, CachedLoudnessEntry>,
}

impl LoudnessCache {
    fn load() -> Self {
        let path = AppConfig::loudness_cache_path();
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<Self>(&raw) {
                return cache;
            }
        }
        Self::default()
    }

    fn save(&self) {
        let path = AppConfig::loudness_cache_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

fn global_cache() -> &'static Mutex<LoudnessCache> {
    static CACHE: OnceLock<Mutex<LoudnessCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LoudnessCache::load()))
}

fn file_cache_key_and_fingerprint(path: &Path) -> (String, String) {
    let key = path.to_string_lossy().to_string();
    if demo_spec_for_path(path).is_some() {
        return (key, "studio-ref-v1".to_string());
    }
    let fp = fs::metadata(path)
        .map(|m| {
            let len = m.len();
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{}", len, mtime)
        })
        .unwrap_or_else(|_| "unknown".to_string());
    (key, fp)
}

pub fn get_cached_loudness(path: &Path) -> Option<TrackLoudness> {
    let (key, fp) = file_cache_key_and_fingerprint(path);
    let guard = global_cache().lock().unwrap();
    guard
        .entries
        .get(&key)
        .filter(|e| e.fingerprint == fp)
        .map(|e| e.loudness.clone())
}

fn put_cached_loudness(path: &Path, loudness: TrackLoudness) {
    let (key, fp) = file_cache_key_and_fingerprint(path);
    let mut guard = global_cache().lock().unwrap();
    guard.entries.insert(
        key,
        CachedLoudnessEntry {
            fingerprint: fp,
            loudness,
        },
    );
    guard.save();
}

pub fn invalidate_cached_loudness(paths: &[PathBuf]) {
    let mut guard = global_cache().lock().unwrap();
    let mut removed = false;
    for p in paths {
        let key = p.to_string_lossy().to_string();
        if guard.entries.remove(&key).is_some() {
            removed = true;
        }
    }
    if removed {
        guard.save();
    }
}

// ── Fast Track Decoder & EBU R128 Scanner ────────────────────────────────────

/// Analyze a single track's ITU-R BS.1770-4 integrated loudness & sample peak.
/// DSD tracks are always normalized at unity (0 dB boost) in cache so toggling
/// the `+6 dB` DSD compensation setting updates loudness instantaneously without rescanning.
pub fn analyze_track_loudness(path: &Path, force_rescan: bool) -> Result<TrackLoudness> {
    if !force_rescan {
        if let Some(cached) = get_cached_loudness(path) {
            return Ok(cached);
        }
    }

    let analysis_dsd = DsdSettings {
        output_mode: DsdOutputMode::Pcm88k,
        cutoff_hz: 30_000,
        gain_boost_6db: false,
    };

    let mut decoder = open_decoder(path, &analysis_dsd)?;
    let props = decoder.properties().clone();
    let sr = props.decoded_sample_rate.max(8_000);
    let ch = props.channels.max(1) as usize;
    let duration = props.duration;

    let mut meter = LoudnessMeter::new(sr, ch);

    // Full exact scan for standard tracks <= 90s; fast 7-window strided scan for long / hi-res / DSD files.
    let use_strided = duration > 90.0
        || props.is_dsd
        || sr > 96_000
        || demo_spec_for_path(path).is_some();

    if use_strided && duration > 18.0 {
        let window_secs = 4.5_f64;
        let window_frames = (window_secs * sr as f64) as usize;
        let fractions = [0.08_f64, 0.22, 0.36, 0.50, 0.64, 0.78, 0.90];
        let mut seek_worked = false;

        for &frac in &fractions {
            let start = (duration * frac).min((duration - window_secs).max(0.0));
            if decoder.seek(start).is_ok() {
                seek_worked = true;
                meter.reset_filter_state();
                let mut collected = 0usize;
                while collected < window_frames {
                    match decoder.next_chunk() {
                        Ok(Some(chunk)) => {
                            let n = chunk.first().map(|c| c.len()).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            meter.push_planar(&chunk);
                            collected += n;
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
            }
        }

        if !seek_worked {
            // Fallback for non-seekable streams: scan up to 45 seconds from start
            let max_frames = (45.0 * sr as f64) as usize;
            let mut collected = 0usize;
            while collected < max_frames {
                match decoder.next_chunk() {
                    Ok(Some(chunk)) => {
                        let n = chunk.first().map(|c| c.len()).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        meter.push_planar(&chunk);
                        collected += n;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
    } else {
        // Full contiguous scan (capped at 120s for safety if duration metadata was missing)
        let max_frames = if duration > 0.0 {
            usize::MAX
        } else {
            (120.0 * sr as f64) as usize
        };
        let mut collected = 0usize;
        while collected < max_frames {
            match decoder.next_chunk() {
                Ok(Some(chunk)) => {
                    let n = chunk.first().map(|c| c.len()).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    meter.push_planar(&chunk);
                    collected += n;
                }
                Ok(None) | Err(_) => break,
            }
        }
    }

    let (lufs, peak) = meter.finalize();
    if !lufs.is_finite() {
        return Err(anyhow!("Loudness measurement produced non-finite result"));
    }

    let result = TrackLoudness {
        integrated_lufs: lufs,
        peak_linear: peak,
        is_dsd: props.is_dsd,
    };
    put_cached_loudness(path, result.clone());
    Ok(result)
}

// ── Gain Computation (AudioGate Song / Album Modes) ──────────────────────────

fn apply_dsd_boost_compensation(m: &TrackLoudness, dsd_settings: &DsdSettings) -> (f32, f32) {
    if m.is_dsd && dsd_settings.gain_boost_6db {
        (m.integrated_lufs + 6.0, m.peak_linear * 1.99526)
    } else {
        (m.integrated_lufs, m.peak_linear)
    }
}

pub fn compute_gain_from_loudness(
    measured_lufs: f32,
    peak_linear: Option<f32>,
    settings: &AutoVolumeSettings,
) -> (f32, Option<f32>, bool) {
    let raw_gain = (settings.target_lufs - measured_lufs).clamp(-24.0, 18.0);
    let peak_dbfs = peak_linear
        .filter(|&p| p > 1e-6)
        .map(|p| 20.0 * p.log10());

    if settings.prevent_clipping {
        if let Some(pk_db) = peak_dbfs {
            let max_safe_gain = PEAK_CEILING_DBFS - pk_db;
            if raw_gain > max_safe_gain {
                return (max_safe_gain.clamp(-24.0, 18.0), peak_dbfs, true);
            }
        }
    }

    (raw_gain, peak_dbfs, false)
}

fn try_resolve_from_tags(
    path: &Path,
    settings: &AutoVolumeSettings,
) -> Option<AutoVolumeResult> {
    if !settings.prefer_replaygain_tags {
        return None;
    }
    let tags = read_replaygain_tags(path);

    match settings.mode {
        AutoVolumeMode::Off => None,
        AutoVolumeMode::Song => {
            if let Some(rg_gain) = tags.track_gain_db {
                let measured_lufs = REPLAYGAIN_REF_LUFS - rg_gain;
                let (gain_db, peak_dbfs, peak_limited) =
                    compute_gain_from_loudness(measured_lufs, tags.track_peak, settings);
                return Some(AutoVolumeResult {
                    mode: AutoVolumeMode::Song,
                    gain_db,
                    measured_lufs,
                    peak_dbfs,
                    peak_limited,
                    source_label: "ReplayGain Track Tag".to_string(),
                });
            }
            if let Some(r128_gain) = tags.r128_track_gain_db {
                let measured_lufs = R128_REF_LUFS - r128_gain;
                let (gain_db, peak_dbfs, peak_limited) =
                    compute_gain_from_loudness(measured_lufs, tags.track_peak, settings);
                return Some(AutoVolumeResult {
                    mode: AutoVolumeMode::Song,
                    gain_db,
                    measured_lufs,
                    peak_dbfs,
                    peak_limited,
                    source_label: "R128 Track Tag".to_string(),
                });
            }
            None
        }
        AutoVolumeMode::Album => {
            if let Some(rg_album_gain) = tags.album_gain_db {
                let measured_lufs = REPLAYGAIN_REF_LUFS - rg_album_gain;
                let peak = tags.album_peak.or(tags.track_peak);
                let (gain_db, peak_dbfs, peak_limited) =
                    compute_gain_from_loudness(measured_lufs, peak, settings);
                return Some(AutoVolumeResult {
                    mode: AutoVolumeMode::Album,
                    gain_db,
                    measured_lufs,
                    peak_dbfs,
                    peak_limited,
                    source_label: "ReplayGain Album Tag".to_string(),
                });
            }
            if let Some(r128_album_gain) = tags.r128_album_gain_db {
                let measured_lufs = R128_REF_LUFS - r128_album_gain;
                let peak = tags.album_peak.or(tags.track_peak);
                let (gain_db, peak_dbfs, peak_limited) =
                    compute_gain_from_loudness(measured_lufs, peak, settings);
                return Some(AutoVolumeResult {
                    mode: AutoVolumeMode::Album,
                    gain_db,
                    measured_lufs,
                    peak_dbfs,
                    peak_limited,
                    source_label: "R128 Album Tag".to_string(),
                });
            }
            None
        }
    }
}

fn aggregate_album_measurements(
    items: &[(f32, f32)], // (integrated_lufs, peak_linear)
) -> (f32, f32) {
    let mut energy_sum = 0.0_f64;
    let mut count = 0usize;
    let mut max_peak = 0.0_f32;

    for &(lufs, peak) in items {
        if lufs > -69.5 && lufs.is_finite() {
            energy_sum += 10.0_f64.powf(((lufs as f64) + 0.691) / 10.0);
            count += 1;
        }
        if peak > max_peak {
            max_peak = peak;
        }
    }

    let album_lufs = if count > 0 {
        let mean_energy = energy_sum / count as f64;
        (-0.691 + 10.0 * mean_energy.log10()) as f32
    } else if let Some(&(first_lufs, _)) = items.first() {
        first_lufs
    } else {
        -16.0
    };

    (album_lufs, max_peak)
}

/// Fast synchronous check (tags + disk/memory cache) — returns `Some` in < 1 ms if no decoding is required.
pub fn try_resolve_auto_volume_fast(
    track_path: &Path,
    album_peers: &[PathBuf],
    settings: &AutoVolumeSettings,
    dsd_settings: &DsdSettings,
) -> Option<AutoVolumeResult> {
    if settings.mode == AutoVolumeMode::Off {
        return None;
    }

    if let Some(from_tags) = try_resolve_from_tags(track_path, settings) {
        return Some(from_tags);
    }

    match settings.mode {
        AutoVolumeMode::Off => None,
        AutoVolumeMode::Song => {
            let cached = get_cached_loudness(track_path)?;
            let (lufs, peak) = apply_dsd_boost_compensation(&cached, dsd_settings);
            let (gain_db, peak_dbfs, peak_limited) =
                compute_gain_from_loudness(lufs, Some(peak), settings);
            Some(AutoVolumeResult {
                mode: AutoVolumeMode::Song,
                gain_db,
                measured_lufs: lufs,
                peak_dbfs,
                peak_limited,
                source_label: "EBU R128 Track".to_string(),
            })
        }
        AutoVolumeMode::Album => {
            let peers = unique_album_paths(track_path, album_peers);
            let mut measurements = Vec::with_capacity(peers.len());
            for p in &peers {
                let cached = get_cached_loudness(p)?;
                measurements.push(apply_dsd_boost_compensation(&cached, dsd_settings));
            }
            let (album_lufs, album_peak) = aggregate_album_measurements(&measurements);
            let (gain_db, peak_dbfs, peak_limited) =
                compute_gain_from_loudness(album_lufs, Some(album_peak), settings);
            let n = measurements.len();
            Some(AutoVolumeResult {
                mode: AutoVolumeMode::Album,
                gain_db,
                measured_lufs: album_lufs,
                peak_dbfs,
                peak_limited,
                source_label: if n > 1 {
                    format!("EBU R128 Album ({} tracks)", n)
                } else {
                    "EBU R128 Album (1 track)".to_string()
                },
            })
        }
    }
}

/// Full resolver (tags → cache → decode/measure via ITU-R BS.1770-4) safe to run on a background thread.
pub fn resolve_auto_volume(
    track_path: &Path,
    album_peers: &[PathBuf],
    settings: &AutoVolumeSettings,
    dsd_settings: &DsdSettings,
    force_rescan: bool,
) -> Result<AutoVolumeResult> {
    if settings.mode == AutoVolumeMode::Off {
        return Ok(AutoVolumeResult {
            mode: AutoVolumeMode::Off,
            gain_db: 0.0,
            measured_lufs: 0.0,
            peak_dbfs: None,
            peak_limited: false,
            source_label: "Off".to_string(),
        });
    }

    if !force_rescan {
        if let Some(from_tags) = try_resolve_from_tags(track_path, settings) {
            return Ok(from_tags);
        }
    }

    match settings.mode {
        AutoVolumeMode::Off => unreachable!(),
        AutoVolumeMode::Song => {
            let m = analyze_track_loudness(track_path, force_rescan)?;
            let (lufs, peak) = apply_dsd_boost_compensation(&m, dsd_settings);
            let (gain_db, peak_dbfs, peak_limited) =
                compute_gain_from_loudness(lufs, Some(peak), settings);
            Ok(AutoVolumeResult {
                mode: AutoVolumeMode::Song,
                gain_db,
                measured_lufs: lufs,
                peak_dbfs,
                peak_limited,
                source_label: "EBU R128 Track".to_string(),
            })
        }
        AutoVolumeMode::Album => {
            let peers = unique_album_paths(track_path, album_peers);
            let mut measurements = Vec::with_capacity(peers.len());
            for p in &peers {
                if let Ok(m) = analyze_track_loudness(p, force_rescan) {
                    measurements.push(apply_dsd_boost_compensation(&m, dsd_settings));
                }
            }
            if measurements.is_empty() {
                return Err(anyhow!("Could not measure album loudness"));
            }
            let (album_lufs, album_peak) = aggregate_album_measurements(&measurements);
            let (gain_db, peak_dbfs, peak_limited) =
                compute_gain_from_loudness(album_lufs, Some(album_peak), settings);
            let n = measurements.len();
            Ok(AutoVolumeResult {
                mode: AutoVolumeMode::Album,
                gain_db,
                measured_lufs: album_lufs,
                peak_dbfs,
                peak_limited,
                source_label: if n > 1 {
                    format!("EBU R128 Album ({} tracks)", n)
                } else {
                    "EBU R128 Album (1 track)".to_string()
                },
            })
        }
    }
}

fn unique_album_paths(track_path: &Path, album_peers: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    out.push(track_path.to_path_buf());
    for p in album_peers {
        if out.len() >= MAX_ALBUM_PEERS_TO_SCAN {
            break;
        }
        if !out.iter().any(|existing| existing == p) {
            out.push(p.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_wav(path: &Path, sample_rate: u32, amp: f32, seconds: f32) {
        let frames = (sample_rate as f32 * seconds) as usize;
        let channels: u16 = 2;
        let data_bytes = (frames * channels as usize * 2) as u32;

        let mut buf = Vec::with_capacity(44 + data_bytes as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        buf.extend_from_slice(b"WAVEfmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
        buf.extend_from_slice(&(channels * 2).to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_bytes.to_le_bytes());

        for i in 0..frames {
            let t = i as f64 / sample_rate as f64;
            let s = (amp as f64 * (2.0 * PI * 997.0 * t).sin()).clamp(-1.0, 1.0);
            let q = (s * i16::MAX as f64).round() as i16;
            buf.extend_from_slice(&q.to_le_bytes());
            buf.extend_from_slice(&q.to_le_bytes());
        }
        fs::write(path, buf).unwrap();
    }

    #[test]
    fn itu_bs1770_calibration_997hz_across_sample_rates() {
        // ITU-R BS.1770-4 reference: 997 Hz stereo sine at -20 dBFS peak (amp = 0.1)
        // yields -20.0 LUFS (±0.15 dB), and mono yields -23.01 LUFS (±0.15 dB).
        for &sr in &[44_100u32, 48_000, 192_000] {
            let frames = sr as usize * 2;
            let amp = 0.1_f32;
            let mut ch = Vec::with_capacity(frames);
            for i in 0..frames {
                let t = i as f64 / sr as f64;
                ch.push((amp as f64 * (2.0 * PI * 997.0 * t).sin()) as f32);
            }

            let mut stereo_meter = LoudnessMeter::new(sr, 2);
            stereo_meter.push_planar(&[ch.clone(), ch.clone()]);
            let (lufs_st, peak_st) = stereo_meter.finalize();
            assert!(
                (lufs_st - (-20.0)).abs() < 0.20,
                "sr={} stereo expected -20.0 LUFS, got {}",
                sr,
                lufs_st
            );
            assert!((peak_st - 0.1).abs() < 0.005);

            let mut mono_meter = LoudnessMeter::new(sr, 1);
            mono_meter.push_planar(&[ch]);
            let (lufs_mono, _) = mono_meter.finalize();
            assert!(
                (lufs_mono - (-23.01)).abs() < 0.20,
                "sr={} mono expected -23.01 LUFS, got {}",
                sr,
                lufs_mono
            );
        }
    }

    #[test]
    fn song_vs_album_mode_preserves_relative_album_dynamics() {
        let tmp = std::env::temp_dir();
        let quiet = tmp.join("sonora_autovol_quiet.wav");
        let loud = tmp.join("sonora_autovol_loud.wav");
        write_test_wav(&quiet, 48_000, 0.05, 1.5); // ~ -26 LUFS
        write_test_wav(&loud, 48_000, 0.40, 1.5); // ~ -8 LUFS

        let dsd = DsdSettings::default();
        let peers = vec![quiet.clone(), loud.clone()];

        // Song mode: each track receives its own independent gain to reach -16 LUFS
        let song_cfg = AutoVolumeSettings {
            mode: AutoVolumeMode::Song,
            target_lufs: -16.0,
            prevent_clipping: false,
            prefer_replaygain_tags: false,
        };
        let res_q_song = resolve_auto_volume(&quiet, &peers, &song_cfg, &dsd, true).unwrap();
        let res_l_song = resolve_auto_volume(&loud, &peers, &song_cfg, &dsd, true).unwrap();
        assert!(res_q_song.gain_db > 8.0, "quiet song should be boosted");
        assert!(res_l_song.gain_db < -6.0, "loud song should be attenuated");
        assert!(
            ((res_q_song.measured_lufs + res_q_song.gain_db)
                - (res_l_song.measured_lufs + res_l_song.gain_db))
                .abs()
                < 0.3,
            "song mode should equalize both tracks to target LUFS"
        );

        // Album mode: both tracks in the same album receive the EXACT same gain,
        // preserving their ~18 dB relative dynamic contrast!
        let album_cfg = AutoVolumeSettings {
            mode: AutoVolumeMode::Album,
            target_lufs: -16.0,
            prevent_clipping: false,
            prefer_replaygain_tags: false,
        };
        let res_q_alb = resolve_auto_volume(&quiet, &peers, &album_cfg, &dsd, false).unwrap();
        let res_l_alb = resolve_auto_volume(&loud, &peers, &album_cfg, &dsd, false).unwrap();
        assert!(
            (res_q_alb.gain_db - res_l_alb.gain_db).abs() < 0.01,
            "album mode must apply identical gain to tracks on the same album ({} vs {})",
            res_q_alb.gain_db,
            res_l_alb.gain_db
        );

        invalidate_cached_loudness(&peers);
        let _ = fs::remove_file(&quiet);
        let _ = fs::remove_file(&loud);
    }

    #[test]
    fn peak_protection_caps_positive_gain_to_prevent_clipping() {
        let cfg = AutoVolumeSettings {
            mode: AutoVolumeMode::Song,
            target_lufs: -14.0,
            prevent_clipping: true,
            prefer_replaygain_tags: false,
        };
        // Very dynamic master: quiet average (-24 LUFS) but high transient peak (-2 dBFS ≈ 0.794)
        // Unprotected gain would be +10 dB (pushing peak to +8 dBFS!).
        // Protected gain must cap at -1.0 - (-2.0) = +1.0 dB.
        let (gain_db, peak_dbfs, limited) = compute_gain_from_loudness(-24.0, Some(0.794328), &cfg);
        assert!(limited);
        assert!((peak_dbfs.unwrap() - (-2.0)).abs() < 0.05);
        assert!((gain_db - 1.0).abs() < 0.05);
    }

    #[test]
    fn replaygain_and_r128_tag_parsing() {
        assert!((parse_db_tag_value("-6.42 dB").unwrap() - (-6.42)).abs() < 1e-4);
        assert!((parse_db_tag_value("+2.35 dB").unwrap() - 2.35).abs() < 1e-4);
        assert!((parse_db_tag_value("0.985412").unwrap() - 0.985412).abs() < 1e-6);
        assert!((parse_r128_q78_db("-512").unwrap() - (-2.0)).abs() < 1e-4);
    }
}
