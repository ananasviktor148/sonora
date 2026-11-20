use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::config::{DsdOutputMode, DsdSettings};

/// Channel layout labels for DSD formats.
#[derive(Debug, Clone)]
pub struct DsdMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lyrics: Option<String>,
    pub album_art: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct DsdFileInfo {
    pub container: &'static str, // "DSF" or "DFF"
    pub dsd_rate: u32,           // e.g. 2_822_400 (DSD64), 5_644_800 (DSD128)...
    pub dsd_label: String,       // e.g. "DSD64", "DSD128", "DSD256", "DSD512"
    pub channels: u16,
    pub channel_labels: Vec<String>,
    pub lsb_first: bool,
    pub total_dsd_samples: u64, // 1-bit samples per channel
    pub duration_secs: f64,
    pub data_offset: u64,
    pub block_size_per_channel: usize, // >0 for DSF, 0 for byte-interleaved DFF
    pub metadata: DsdMetadata,
}

pub fn dsd_rate_label(dsd_rate: u32) -> String {
    let base44 = 44_100u32;
    let base48 = 48_000u32;
    if dsd_rate.is_multiple_of(base44) {
        format!("DSD{}", dsd_rate / base44)
    } else if dsd_rate.is_multiple_of(base48) {
        format!("DSD{} (48k)", dsd_rate / base48)
    } else {
        format!("DSD ({:.2} MHz)", dsd_rate as f64 / 1_000_000.0)
    }
}

pub fn is_dsd_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("dsf") || e.eq_ignore_ascii_case("dff"))
        .unwrap_or(false)
}

/// Inspect a DSF or DFF file header (and embedded ID3v2 tags) without reading the audio payload.
pub fn probe_dsd_file(path: &Path) -> Result<DsdFileInfo> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;

    match &magic {
        b"DSD " => parse_dsf_header(&mut file),
        b"FRM8" => parse_dff_header(&mut file),
        _ => bail!("Not a valid DSF or DSDIFF (DFF) file"),
    }
}

fn dsf_channel_labels(channel_type: u32, channel_count: u16) -> Vec<String> {
    match channel_type {
        1 => vec!["M".into()],
        2 => vec!["FL".into(), "FR".into()],
        3 => vec!["FL".into(), "FR".into(), "FC".into()],
        4 => vec!["FL".into(), "FR".into(), "BL".into(), "BR".into()],
        5 => vec!["FL".into(), "FR".into(), "FC".into(), "LFE".into()],
        6 => vec![
            "FL".into(),
            "FR".into(),
            "FC".into(),
            "BL".into(),
            "BR".into(),
        ],
        7 => vec![
            "FL".into(),
            "FR".into(),
            "FC".into(),
            "LFE".into(),
            "BL".into(),
            "BR".into(),
        ],
        _ => default_channel_labels(channel_count),
    }
}

pub fn default_channel_labels(channels: u16) -> Vec<String> {
    match channels {
        1 => vec!["M".into()],
        2 => vec!["FL".into(), "FR".into()],
        3 => vec!["FL".into(), "FR".into(), "FC".into()],
        4 => vec!["FL".into(), "FR".into(), "SL".into(), "SR".into()],
        5 => vec![
            "FL".into(),
            "FR".into(),
            "FC".into(),
            "SL".into(),
            "SR".into(),
        ],
        6 => vec![
            "FL".into(),
            "FR".into(),
            "FC".into(),
            "LFE".into(),
            "SL".into(),
            "SR".into(),
        ],
        7 => vec![
            "FL".into(),
            "FR".into(),
            "FC".into(),
            "LFE".into(),
            "BC".into(),
            "SL".into(),
            "SR".into(),
        ],
        8 => vec![
            "FL".into(),
            "FR".into(),
            "FC".into(),
            "LFE".into(),
            "BL".into(),
            "BR".into(),
            "SL".into(),
            "SR".into(),
        ],
        n => (0..n).map(|i| format!("CH{}", i + 1)).collect(),
    }
}

fn parse_dsf_header(file: &mut File) -> Result<DsdFileInfo> {
    // "DSD " (4 bytes) already read
    let chunk_size = read_u64_le(file)?;
    let _total_file_size = read_u64_le(file)?;
    let metadata_offset = read_u64_le(file)?;

    if chunk_size > 28 {
        file.seek(SeekFrom::Current((chunk_size - 28) as i64))?;
    }

    // "fmt " chunk
    let mut fmt_magic = [0u8; 4];
    file.read_exact(&mut fmt_magic)?;
    if &fmt_magic != b"fmt " {
        bail!("DSF: expected 'fmt ' chunk, found {:?}", fmt_magic);
    }

    let fmt_chunk_size = read_u64_le(file)?;
    let _format_version = read_u32_le(file)?;
    let format_id = read_u32_le(file)?;
    if format_id != 0 {
        bail!("DSF: unsupported format_id {} (only raw DSD=0 supported)", format_id);
    }
    let channel_type = read_u32_le(file)?;
    let channel_count = read_u32_le(file)? as u16;
    let dsd_rate = read_u32_le(file)?;
    let bits_per_sample = read_u32_le(file)?;
    let sample_count = read_u64_le(file)?;
    let block_size_per_channel = read_u32_le(file)? as usize;
    let _reserved = read_u32_le(file)?;

    if channel_count == 0 || dsd_rate == 0 || block_size_per_channel == 0 {
        bail!("DSF: invalid header parameters");
    }

    let fmt_read = 52u64;
    if fmt_chunk_size > fmt_read {
        file.seek(SeekFrom::Current((fmt_chunk_size - fmt_read) as i64))?;
    }

    // "data" chunk
    let mut data_magic = [0u8; 4];
    file.read_exact(&mut data_magic)?;
    if &data_magic != b"data" {
        bail!("DSF: expected 'data' chunk, found {:?}", data_magic);
    }
    let data_chunk_size = read_u64_le(file)?;
    let data_offset = file.stream_position()?;
    let _data_size = data_chunk_size.saturating_sub(12);

    // DSF spec: bits_per_sample == 1 means LSB-first, 8 means MSB-first
    let lsb_first = bits_per_sample == 1;

    // Parse embedded ID3v2 metadata if present
    let metadata = if metadata_offset > 0 {
        parse_id3v2_at(file, metadata_offset).unwrap_or(DsdMetadata {
            title: None,
            artist: None,
            album: None,
            lyrics: None,
            album_art: None,
        })
    } else {
        DsdMetadata {
            title: None,
            artist: None,
            album: None,
            lyrics: None,
            album_art: None,
        }
    };

    let duration_secs = if dsd_rate > 0 {
        sample_count as f64 / dsd_rate as f64
    } else {
        0.0
    };

    Ok(DsdFileInfo {
        container: "DSF",
        dsd_rate,
        dsd_label: dsd_rate_label(dsd_rate),
        channels: channel_count,
        channel_labels: dsf_channel_labels(channel_type, channel_count),
        lsb_first,
        total_dsd_samples: sample_count,
        duration_secs,
        data_offset,
        block_size_per_channel,
        metadata,
    })
}

fn parse_dff_header(file: &mut File) -> Result<DsdFileInfo> {
    // "FRM8" (4 bytes) already read
    let frm8_size = read_u64_be(file)?;
    let frm8_end = 12u64.saturating_add(frm8_size);

    let mut form_type = [0u8; 4];
    file.read_exact(&mut form_type)?;
    if &form_type != b"DSD " {
        bail!("DFF: expected 'DSD ' form type");
    }

    let mut dsd_rate: u32 = 2_822_400;
    let mut channels: u16 = 2;
    let mut channel_labels: Vec<String> = Vec::new();
    let mut compression = *b"DSD ";
    let mut data_offset: u64 = 0;
    let mut data_size: u64 = 0;
    let mut metadata = DsdMetadata {
        title: None,
        artist: None,
        album: None,
        lyrics: None,
        album_art: None,
    };

    while file.stream_position()? + 12 <= frm8_end {
        let mut chunk_id = [0u8; 4];
        if file.read_exact(&mut chunk_id).is_err() {
            break;
        }
        let chunk_size = match read_u64_be(file) {
            Ok(s) => s,
            Err(_) => break,
        };
        let chunk_start = file.stream_position()?;
        let padded_size = chunk_size + (chunk_size & 1);

        match &chunk_id {
            b"PROP" => {
                let mut prop_type = [0u8; 4];
                file.read_exact(&mut prop_type)?;
                if &prop_type == b"SND " {
                    let prop_end = chunk_start + chunk_size;
                    while file.stream_position()? + 12 <= prop_end {
                        let mut sub_id = [0u8; 4];
                        if file.read_exact(&mut sub_id).is_err() {
                            break;
                        }
                        let sub_size = read_u64_be(file)?;
                        let sub_start = file.stream_position()?;
                        let sub_padded = sub_size + (sub_size & 1);

                        match &sub_id {
                            b"FS  " if sub_size >= 4 => {
                                dsd_rate = read_u32_be(file)?;
                            }
                            b"CHNL" if sub_size >= 2 => {
                                channels = read_u16_be(file)?;
                                channel_labels.clear();
                                for _ in 0..channels {
                                    let mut cid = [0u8; 4];
                                    if file.read_exact(&mut cid).is_ok() {
                                        let label = match &cid {
                                            b"SLFT" | b"MLFT" => "FL",
                                            b"SRGT" | b"MRGT" => "FR",
                                            b"C   " => "FC",
                                            b"LFE " => "LFE",
                                            b"LS  " => "SL",
                                            b"RS  " => "SR",
                                            _ => "CH",
                                        };
                                        channel_labels.push(label.to_string());
                                    }
                                }
                            }
                            b"CMPR" if sub_size >= 4 => {
                                file.read_exact(&mut compression)?;
                            }
                            _ => {}
                        }
                        file.seek(SeekFrom::Start(sub_start + sub_padded))?;
                    }
                }
            }
            b"DSD " => {
                data_offset = chunk_start;
                data_size = chunk_size;
            }
            b"DST " => {
                compression = *b"DST ";
            }
            b"DIIN" => {
                let diin_end = chunk_start + chunk_size;
                while file.stream_position()? + 12 <= diin_end {
                    let mut sub_id = [0u8; 4];
                    if file.read_exact(&mut sub_id).is_err() {
                        break;
                    }
                    let sub_size = read_u64_be(file)?;
                    let sub_start = file.stream_position()?;
                    let sub_padded = sub_size + (sub_size & 1);
                    if (&sub_id == b"DIAR" || &sub_id == b"DITI") && sub_size >= 4 {
                        let str_len = read_u32_be(file)? as usize;
                        let mut buf = vec![0u8; str_len.min(sub_size as usize - 4)];
                        if file.read_exact(&mut buf).is_ok() {
                            let text = String::from_utf8_lossy(&buf).trim().to_string();
                            if !text.is_empty() {
                                if &sub_id == b"DIAR" && metadata.artist.is_none() {
                                    metadata.artist = Some(text);
                                } else if &sub_id == b"DITI" && metadata.title.is_none() {
                                    metadata.title = Some(text);
                                }
                            }
                        }
                    }
                    file.seek(SeekFrom::Start(sub_start + sub_padded))?;
                }
            }
            b"ID3 " => {
                if let Ok(id3_meta) = parse_id3v2_at(file, chunk_start) {
                    if id3_meta.title.is_some() {
                        metadata.title = id3_meta.title;
                    }
                    if id3_meta.artist.is_some() {
                        metadata.artist = id3_meta.artist;
                    }
                    if id3_meta.album.is_some() {
                        metadata.album = id3_meta.album;
                    }
                    if id3_meta.lyrics.is_some() {
                        metadata.lyrics = id3_meta.lyrics;
                    }
                    if id3_meta.album_art.is_some() {
                        metadata.album_art = id3_meta.album_art;
                    }
                }
            }
            _ => {}
        }

        file.seek(SeekFrom::Start(chunk_start + padded_size))?;
    }

    if &compression == b"DST " {
        bail!("DST-compressed DFF requires FFmpeg fallback");
    }
    if data_size == 0 || channels == 0 || dsd_rate == 0 {
        bail!("DFF: missing or invalid DSD data chunk");
    }

    if channel_labels.len() != channels as usize {
        channel_labels = default_channel_labels(channels);
    }

    let bytes_per_channel = data_size / channels as u64;
    let total_dsd_samples = bytes_per_channel * 8;
    let duration_secs = total_dsd_samples as f64 / dsd_rate as f64;

    Ok(DsdFileInfo {
        container: "DFF",
        dsd_rate,
        dsd_label: dsd_rate_label(dsd_rate),
        channels,
        channel_labels,
        lsb_first: false, // DFF is always MSB-first
        total_dsd_samples,
        duration_secs,
        data_offset,
        block_size_per_channel: 0,
        metadata,
    })
}

// ── Byte-LUT Polyphase FIR DSD-to-PCM Decimator ──────────────────────────────

/// High-precision DSD-to-PCM decimator using a linear-phase Blackman-Harris windowed
/// sinc FIR filter accelerated via 256-entry byte lookup tables (8 DSD bits per lookup).
pub struct DsdToPcmDecimator {
    channels: usize,
    bytes_per_step: usize,
    filter_bytes: usize,
    /// `lut[byte_idx][byte_val]` = weighted sum of the 8 DSD bits at `byte_idx`.
    lut: Vec<[f32; 256]>,
    /// Per-channel sliding window of the most recent `filter_bytes` DSD bytes.
    history: Vec<Vec<u8>>,
    /// Per-channel residual input byte accumulator when input chunks aren't a multiple of `bytes_per_step`.
    pending: Vec<Vec<u8>>,
    pub output_rate: u32,
}

impl DsdToPcmDecimator {
    pub fn new(
        dsd_rate: u32,
        channels: usize,
        lsb_first: bool,
        settings: &DsdSettings,
    ) -> Result<Self> {
        let desired_pcm = settings.output_mode.target_pcm_rate().unwrap_or(176_400);
        // Compute integer decimation factor divisible by 8 (1 DSD byte = 8 bits)
        let mut decimation = (dsd_rate / desired_pcm).max(8);
        decimation = ((decimation + 4) / 8) * 8;
        if decimation < 8 {
            decimation = 8;
        }
        // Ensure exact integer division where possible
        while !dsd_rate.is_multiple_of(decimation) && decimation > 8 {
            decimation -= 8;
        }

        let output_rate = dsd_rate / decimation;
        let bytes_per_step = (decimation / 8) as usize;

        // Safe cutoff below both user preference and output Nyquist (with transition margin)
        let max_safe_cutoff = (output_rate as f64) * 0.42;
        let cutoff_hz = (settings.cutoff_hz as f64).min(max_safe_cutoff).max(18_000.0);

        // Choose filter length (in bytes) proportional to dsd_rate/cutoff for steep >92 dB ultrasonic rejection
        let ideal_taps = ((dsd_rate as f64 / cutoff_hz) * 3.5).ceil() as usize;
        let filter_bytes = ideal_taps
            .div_ceil(8)
            .max(bytes_per_step * 2)
            .clamp(16, 96);
        let num_taps = filter_bytes * 8;

        // Design linear-phase low-pass sinc FIR with 4-term Blackman-Harris window
        let fc = cutoff_hz / (dsd_rate as f64);
        let center = (num_taps - 1) as f64 * 0.5;
        let mut taps = vec![0.0f64; num_taps];
        let mut sum = 0.0f64;

        for (n, tap) in taps.iter_mut().enumerate() {
            let x = n as f64 - center;
            let sinc = if x.abs() < 1e-12 {
                2.0 * fc
            } else {
                (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x)
            };
            let w_arg = 2.0 * std::f64::consts::PI * (n as f64) / ((num_taps - 1) as f64);
            let window = 0.35875
                - 0.48829 * w_arg.cos()
                + 0.14128 * (2.0 * w_arg).cos()
                - 0.01168 * (3.0 * w_arg).cos();
            *tap = sinc * window;
            sum += *tap;
        }

        // Normalize unity DC gain, plus optional +6 dB SACD gain compensation (1.995x)
        let gain = if settings.gain_boost_6db { 1.99526 } else { 1.0 };
        let norm = gain / sum;
        for t in &mut taps {
            *t *= norm;
        }

        // Precompute byte lookup tables for instant 8-bits-at-once convolution
        let mut lut = vec![[0.0f32; 256]; filter_bytes];
        for (byte_idx, byte_lut) in lut.iter_mut().enumerate() {
            let base_tap = byte_idx * 8;
            for (byte_val, slot) in byte_lut.iter_mut().enumerate() {
                let mut acc = 0.0f64;
                for bit in 0..8 {
                    let b = if lsb_first {
                        (byte_val >> bit) & 1
                    } else {
                        (byte_val >> (7 - bit)) & 1
                    };
                    let val = if b == 1 { 1.0 } else { -1.0 };
                    acc += val * taps[base_tap + bit];
                }
                *slot = acc as f32;
            }
        }

        // Initialize history with DSD silence pattern (0x69 = 01101001, zero DC)
        let history = vec![vec![0x69u8; filter_bytes]; channels];
        let pending = vec![Vec::new(); channels];

        Ok(Self {
            channels,
            bytes_per_step,
            filter_bytes,
            lut,
            history,
            pending,
            output_rate,
        })
    }

    pub fn reset(&mut self) {
        for h in &mut self.history {
            h.fill(0x69);
        }
        for p in &mut self.pending {
            p.clear();
        }
    }

    /// Process planar DSD byte slices (one slice per channel, equal length) into planar PCM `f32`.
    pub fn process(&mut self, dsd_channels: &[Vec<u8>]) -> Vec<Vec<f32>> {
        let mut out = Vec::with_capacity(self.channels);

        for ch in 0..self.channels {
            if ch >= dsd_channels.len() {
                out.push(Vec::new());
                continue;
            }
            self.pending[ch].extend_from_slice(&dsd_channels[ch]);
            let avail = self.pending[ch].len();
            let steps = avail / self.bytes_per_step;
            let consumed = steps * self.bytes_per_step;

            let mut pcm = Vec::with_capacity(steps);
            let hist = &mut self.history[ch];
            let fb = self.filter_bytes;
            let step = self.bytes_per_step;

            for s in 0..steps {
                let src = &self.pending[ch][s * step..(s + 1) * step];
                if step >= fb {
                    hist.copy_from_slice(&src[step - fb..]);
                } else {
                    hist.copy_within(step..fb, 0);
                    hist[fb - step..].copy_from_slice(src);
                }

                let mut acc = 0.0f32;
                for (b_idx, &byte_val) in hist.iter().enumerate() {
                    acc += self.lut[b_idx][byte_val as usize];
                }
                pcm.push(acc);
            }

            if consumed > 0 {
                self.pending[ch].drain(..consumed);
            }
            out.push(pcm);
        }

        out
    }
}

// ── DoP v1.1 (DSD over PCM) Packer ───────────────────────────────────────────

/// Packs raw DSD bitstream bytes into DoP v1.1 24-bit PCM samples normalized to `f32`
/// (exact 24-bit integer round-trip in IEEE-754 single precision).
pub struct DopPacker {
    channels: usize,
    lsb_first: bool,
    marker_toggle: bool,
    pending: Vec<Vec<u8>>,
    pub output_rate: u32,
}

impl DopPacker {
    pub fn new(dsd_rate: u32, channels: usize, lsb_first: bool) -> Self {
        Self {
            channels,
            lsb_first,
            marker_toggle: false,
            pending: vec![Vec::new(); channels],
            output_rate: dsd_rate / 16, // 16 DSD bits per DoP PCM frame
        }
    }

    pub fn reset(&mut self) {
        self.marker_toggle = false;
        for p in &mut self.pending {
            p.clear();
        }
    }

    pub fn process(&mut self, dsd_channels: &[Vec<u8>]) -> Vec<Vec<f32>> {
        let mut min_pairs = usize::MAX;
        for ch in 0..self.channels {
            if ch < dsd_channels.len() {
                self.pending[ch].extend_from_slice(&dsd_channels[ch]);
            }
            min_pairs = min_pairs.min(self.pending[ch].len() / 2);
        }
        if min_pairs == usize::MAX || min_pairs == 0 {
            return vec![Vec::new(); self.channels];
        }

        let mut out = vec![Vec::with_capacity(min_pairs); self.channels];
        for frame in 0..min_pairs {
            let marker: u32 = if self.marker_toggle { 0xFA } else { 0x05 };
            self.marker_toggle = !self.marker_toggle;

            for (out_ch, pend_ch) in out.iter_mut().zip(self.pending.iter()) {
                let mut b0 = pend_ch[frame * 2];
                let mut b1 = pend_ch[frame * 2 + 1];
                if self.lsb_first {
                    b0 = b0.reverse_bits();
                    b1 = b1.reverse_bits();
                }
                let raw24: u32 = (marker << 16) | ((b0 as u32) << 8) | (b1 as u32);
                // Sign-extend 24-bit value to i32 (0x05xxxxxx is positive, 0xFAxxxxxx is negative)
                let signed24: i32 = ((raw24 << 8) as i32) >> 8;
                let sample = (signed24 as f32) / 8_388_608.0;
                out_ch.push(sample);
            }
        }

        let consumed = min_pairs * 2;
        for ch in 0..self.channels {
            self.pending[ch].drain(..consumed);
        }
        out
    }
}

// ── Streaming DSD Reader ─────────────────────────────────────────────────────

enum DsdProcessor {
    Pcm(DsdToPcmDecimator),
    DoP(DopPacker),
}

/// Real-time streaming DSF/DFF decoder supporting constant-memory playback and instant seeking.
pub struct DsdStreamReader {
    file: BufReader<File>,
    pub info: DsdFileInfo,
    processor: DsdProcessor,
    dsd_bytes_read_per_ch: u64,
    total_valid_bytes_per_ch: u64,
    pub output_sample_rate: u32,
    pub is_dop: bool,
}

impl DsdStreamReader {
    pub fn open(path: &Path, settings: &DsdSettings) -> Result<Self> {
        let info = probe_dsd_file(path)?;
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(info.data_offset))?;

        let (processor, output_sample_rate, is_dop) = match settings.output_mode {
            DsdOutputMode::DoP => {
                let dop = DopPacker::new(info.dsd_rate, info.channels as usize, info.lsb_first);
                let rate = dop.output_rate;
                (DsdProcessor::DoP(dop), rate, true)
            }
            _ => {
                let dec = DsdToPcmDecimator::new(
                    info.dsd_rate,
                    info.channels as usize,
                    info.lsb_first,
                    settings,
                )?;
                let rate = dec.output_rate;
                (DsdProcessor::Pcm(dec), rate, false)
            }
        };

        let total_valid_bytes_per_ch = info.total_dsd_samples.div_ceil(8);

        Ok(Self {
            file: BufReader::with_capacity(64 * 1024, file),
            info,
            processor,
            dsd_bytes_read_per_ch: 0,
            total_valid_bytes_per_ch,
            output_sample_rate,
            is_dop,
        })
    }

    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        let clamped_sec = seconds.clamp(0.0, self.info.duration_secs);
        let target_dsd_byte = ((clamped_sec * self.info.dsd_rate as f64) / 8.0) as u64;
        let target_dsd_byte = target_dsd_byte.min(self.total_valid_bytes_per_ch);
        let ch = self.info.channels as u64;

        if self.info.block_size_per_channel > 0 {
            // DSF: align to block boundary
            let bs = self.info.block_size_per_channel as u64;
            let block_idx = target_dsd_byte / bs;
            let file_pos = self.info.data_offset + block_idx * bs * ch;
            self.file.seek(SeekFrom::Start(file_pos))?;
            self.dsd_bytes_read_per_ch = block_idx * bs;
        } else {
            // DFF: byte-interleaved per channel
            let file_pos = self.info.data_offset + target_dsd_byte * ch;
            self.file.seek(SeekFrom::Start(file_pos))?;
            self.dsd_bytes_read_per_ch = target_dsd_byte;
        }

        match &mut self.processor {
            DsdProcessor::Pcm(dec) => dec.reset(),
            DsdProcessor::DoP(dop) => dop.reset(),
        }

        Ok(())
    }

    /// Read and convert the next chunk of DSD data to planar `f32` PCM (or DoP frames).
    /// Returns `Ok(None)` at EOF.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>> {
        if self.dsd_bytes_read_per_ch >= self.total_valid_bytes_per_ch {
            return Ok(None);
        }

        let ch_count = self.info.channels as usize;
        let remaining_bytes_per_ch =
            (self.total_valid_bytes_per_ch - self.dsd_bytes_read_per_ch) as usize;

        let channel_dsd: Vec<Vec<u8>> = if self.info.block_size_per_channel > 0 {
            // DSF: read one full interleaved block across all channels
            let bs = self.info.block_size_per_channel;
            let block_total = bs * ch_count;
            let mut raw = vec![0u8; block_total];
            let n = read_up_to(&mut self.file, &mut raw)?;
            if n < ch_count {
                return Ok(None);
            }
            let valid_per_ch = bs.min(remaining_bytes_per_ch);
            let mut per_ch = vec![Vec::with_capacity(valid_per_ch); ch_count];
            for (c, ch_buf) in per_ch.iter_mut().enumerate().take(ch_count) {
                let start = c * bs;
                let end = (start + valid_per_ch).min(n);
                if start < end {
                    ch_buf.extend_from_slice(&raw[start..end]);
                }
            }
            self.dsd_bytes_read_per_ch += valid_per_ch as u64;
            per_ch
        } else {
            // DFF: read ~4096 bytes per channel, byte-interleaved
            let frames_to_read = 4096usize.min(remaining_bytes_per_ch);
            let mut raw = vec![0u8; frames_to_read * ch_count];
            let n = read_up_to(&mut self.file, &mut raw)?;
            let actual_frames = n / ch_count;
            if actual_frames == 0 {
                return Ok(None);
            }
            let mut per_ch = vec![Vec::with_capacity(actual_frames); ch_count];
            for f in 0..actual_frames {
                let base = f * ch_count;
                for c in 0..ch_count {
                    per_ch[c].push(raw[base + c]);
                }
            }
            self.dsd_bytes_read_per_ch += actual_frames as u64;
            per_ch
        };

        let pcm = match &mut self.processor {
            DsdProcessor::Pcm(dec) => dec.process(&channel_dsd),
            DsdProcessor::DoP(dop) => dop.process(&channel_dsd),
        };

        if pcm.first().map(|c| c.is_empty()).unwrap_or(true) {
            if self.dsd_bytes_read_per_ch >= self.total_valid_bytes_per_ch {
                return Ok(None);
            }
            // Need more DSD bytes to emit the first decimated sample; recurse cleanly
            return self.next_chunk();
        }

        Ok(Some(pcm))
    }
}

// ── Minimal, reliable ID3v2.3 / ID3v2.4 parser for DSF / DFF embedded tags ──

fn parse_id3v2_at(file: &mut File, offset: u64) -> Result<DsdMetadata> {
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; 10];
    file.read_exact(&mut header)?;
    if &header[0..3] != b"ID3" {
        return Err(anyhow!("No ID3v2 header at offset {}", offset));
    }
    let major_version = header[3]; // 3 or 4
    let tag_size = decode_synchsafe(&header[6..10]) as usize;
    let mut data = vec![0u8; tag_size.min(32 * 1024 * 1024)];
    let n = read_up_to(file, &mut data)?;
    data.truncate(n);

    let mut meta = DsdMetadata {
        title: None,
        artist: None,
        album: None,
        lyrics: None,
        album_art: None,
    };

    let mut pos = 0usize;
    while pos + 10 <= data.len() {
        let frame_id = &data[pos..pos + 4];
        if frame_id[0] == 0 {
            break;
        }
        let frame_size = if major_version >= 4 {
            decode_synchsafe(&data[pos + 4..pos + 8]) as usize
        } else {
            u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                as usize
        };
        pos += 10;
        if frame_size == 0 || pos + frame_size > data.len() {
            break;
        }
        let payload = &data[pos..pos + frame_size];
        pos += frame_size;

        match frame_id {
            b"TIT2" => meta.title = decode_id3_text(payload),
            b"TPE1" => meta.artist = decode_id3_text(payload),
            b"TALB" => meta.album = decode_id3_text(payload),
            b"USLT" => meta.lyrics = decode_id3_uslt(payload),
            b"APIC" if meta.album_art.is_none() => {
                meta.album_art = decode_id3_apic(payload);
            }
            _ => {}
        }
    }

    Ok(meta)
}

fn decode_synchsafe(b: &[u8]) -> u32 {
    ((b[0] as u32 & 0x7F) << 21)
        | ((b[1] as u32 & 0x7F) << 14)
        | ((b[2] as u32 & 0x7F) << 7)
        | (b[3] as u32 & 0x7F)
}

fn decode_id3_string(encoding: u8, bytes: &[u8]) -> String {
    match encoding {
        0 => bytes
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect(),
        1 => decode_utf16_with_bom(bytes),
        2 => {
            let u16s: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_be_bytes(c))
                .take_while(|&v| v != 0)
                .collect();
            String::from_utf16_lossy(&u16s)
        }
        3 => {
            let trimmed = bytes.split(|&b| b == 0).next().unwrap_or(bytes);
            String::from_utf8_lossy(trimmed).into_owned()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
    .trim()
    .to_string()
}

fn decode_utf16_with_bom(bytes: &[u8]) -> String {
    if bytes.len() < 2 {
        return String::new();
    }
    let (le, rest) = if bytes[0] == 0xFF && bytes[1] == 0xFE {
        (true, &bytes[2..])
    } else if bytes[0] == 0xFE && bytes[1] == 0xFF {
        (false, &bytes[2..])
    } else {
        (true, bytes)
    };
    let u16s: Vec<u16> = rest
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| {
            if le {
                u16::from_le_bytes(c)
            } else {
                u16::from_be_bytes(c)
            }
        })
        .take_while(|&v| v != 0)
        .collect();
    String::from_utf16_lossy(&u16s)
}

fn decode_id3_text(payload: &[u8]) -> Option<String> {
    if payload.is_empty() {
        return None;
    }
    let s = decode_id3_string(payload[0], &payload[1..]);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn decode_id3_uslt(payload: &[u8]) -> Option<String> {
    // [encoding: 1][lang: 3][content_desc: null-terminated][lyrics text]
    if payload.len() < 5 {
        return None;
    }
    let enc = payload[0];
    let rest = &payload[4..];
    let text_start = if enc == 1 || enc == 2 {
        let mut i = 0;
        while i + 1 < rest.len() {
            if rest[i] == 0 && rest[i + 1] == 0 {
                i += 2;
                break;
            }
            i += 2;
        }
        i
    } else {
        rest.iter()
            .position(|&b| b == 0)
            .map(|p| p + 1)
            .unwrap_or(0)
    };
    if text_start >= rest.len() {
        return None;
    }
    let text = decode_id3_string(enc, &rest[text_start..]);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn decode_id3_apic(payload: &[u8]) -> Option<Vec<u8>> {
    // [encoding: 1][mime: null-term][pic_type: 1][desc: null-term][image data]
    if payload.len() < 6 {
        return None;
    }
    let enc = payload[0];
    let mime_end = payload[1..].iter().position(|&b| b == 0)? + 1;
    let desc_start = mime_end + 2; // skip null + pic_type
    if desc_start >= payload.len() {
        return None;
    }
    let rest = &payload[desc_start..];
    let img_offset = if enc == 1 || enc == 2 {
        let mut i = 0;
        while i + 1 < rest.len() {
            if rest[i] == 0 && rest[i + 1] == 0 {
                i += 2;
                break;
            }
            i += 2;
        }
        i
    } else {
        rest.iter().position(|&b| b == 0)? + 1
    };
    if img_offset >= rest.len() {
        return None;
    }
    Some(rest[img_offset..].to_vec())
}

// ── Binary helpers ───────────────────────────────────────────────────────────

fn read_up_to<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(total)
}

fn read_u16_be(file: &mut File) -> Result<u16> {
    let mut b = [0u8; 2];
    file.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32_le(file: &mut File) -> Result<u32> {
    let mut b = [0u8; 4];
    file.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u32_be(file: &mut File) -> Result<u32> {
    let mut b = [0u8; 4];
    file.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn read_u64_le(file: &mut File) -> Result<u64> {
    let mut b = [0u8; 8];
    file.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_u64_be(file: &mut File) -> Result<u64> {
    let mut b = [0u8; 8];
    file.read_exact(&mut b)?;
    Ok(u64::from_be_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Generate a 2nd-order sigma-delta modulated DSD bitstream from a sine wave.
    fn sigma_delta_sine(freq_hz: f64, dsd_rate: u32, num_bytes: usize, lsb_first: bool) -> Vec<u8> {
        let mut out = vec![0u8; num_bytes];
        let mut err1 = 0.0f64;
        let mut err2 = 0.0f64;
        let mut y_prev = 0.0f64;
        let amp = 0.45; // standard SACD modulation depth

        for (byte_idx, out_byte) in out.iter_mut().enumerate() {
            let mut b = 0u8;
            for bit in 0..8 {
                let sample_idx = byte_idx * 8 + bit;
                let t = sample_idx as f64 / dsd_rate as f64;
                let x = amp * (2.0 * std::f64::consts::PI * freq_hz * t).sin();
                err1 += x - y_prev;
                err2 += err1 - y_prev;
                let out_bit = if err2 >= 0.0 {
                    y_prev = 1.0;
                    1u8
                } else {
                    y_prev = -1.0;
                    0u8
                };
                if out_bit == 1 {
                    if lsb_first {
                        b |= 1 << bit;
                    } else {
                        b |= 1 << (7 - bit);
                    }
                }
            }
            *out_byte = b;
        }
        out
    }

    #[test]
    fn dsd64_decimator_reconstructs_clean_sine() {
        let dsd_rate = 2_822_400u32;
        let num_bytes = 35_280; // 100 ms of DSD64
        let dsd = sigma_delta_sine(1000.0, dsd_rate, num_bytes, true);

        let settings = DsdSettings {
            output_mode: DsdOutputMode::Pcm176k,
            cutoff_hz: 30_000,
            gain_boost_6db: false,
        };
        let mut dec = DsdToPcmDecimator::new(dsd_rate, 1, true, &settings).unwrap();
        assert_eq!(dec.output_rate, 176_400);

        let pcm = dec.process(&[dsd]);
        let samples = &pcm[0];
        assert!(samples.len() > 10_000);

        // Measure 1 kHz tone energy vs residual error after filter settling
        let steady = &samples[200..samples.len() - 200];
        let mut sin_acc = 0.0f64;
        let mut cos_acc = 0.0f64;
        let mut total_energy = 0.0f64;
        for (i, &s) in steady.iter().enumerate() {
            let ph = 2.0 * std::f64::consts::PI * 1000.0 * (i as f64) / 176_400.0;
            sin_acc += (s as f64) * ph.sin();
            cos_acc += (s as f64) * ph.cos();
            total_energy += (s as f64) * (s as f64);
        }
        let n = steady.len() as f64;
        let amp_est = 2.0 * (sin_acc.hypot(cos_acc)) / n;
        let sig_energy = 0.5 * amp_est * amp_est * n;
        let noise_energy = (total_energy - sig_energy).max(1e-12);
        let snr_db = 10.0 * (sig_energy / noise_energy).log10();

        assert!(
            (amp_est - 0.45).abs() < 0.04,
            "Expected ~0.45 amplitude, got {}",
            amp_est
        );
        assert!(snr_db > 45.0, "Expected high SNR, got {:.1} dB", snr_db);
    }

    #[test]
    fn dop_packer_alternates_markers_and_reverses_lsb() {
        let mut dop = DopPacker::new(2_822_400, 2, true);
        assert_eq!(dop.output_rate, 176_400);

        let ch0 = vec![0x80, 0x01, 0xAA, 0x55];
        let ch1 = vec![0xFF, 0x00, 0x0F, 0xF0];
        let out = dop.process(&[ch0, ch1]);
        assert_eq!(out[0].len(), 2);

        // Recover exact 24-bit integer from f32
        let f0 = (out[0][0] * 8_388_608.0).round() as i32 & 0x00FF_FFFF;
        let f1 = (out[0][1] * 8_388_608.0).round() as i32 & 0x00FF_FFFF;
        assert_eq!((f0 >> 16) & 0xFF, 0x05);
        assert_eq!((f0 >> 8) & 0xFF, 0x80u8.reverse_bits() as i32);
        assert_eq!(f0 & 0xFF, 0x01u8.reverse_bits() as i32);
        assert_eq!((f1 >> 16) & 0xFF, 0xFA);
    }

    #[test]
    fn dsf_and_dff_round_trip_multichannel_and_seek() {
        let tmp = std::env::temp_dir();
        let dsf_path = tmp.join("test_hires_player_51.dsf");
        let channels = 6u32;
        let block_size = 4096usize;
        let blocks = 4usize;
        let total_samples = (blocks * block_size * 8) as u64;
        let data_payload_size = (blocks * block_size * channels as usize) as u64;

        // Write a valid 5.1 DSF file with an ID3v2 title tag
        {
            let mut f = File::create(&dsf_path).unwrap();
            let id3_offset = 28 + 52 + 12 + data_payload_size;
            let mut id3 = Vec::new();
            let title_frame_data = b"\x03Surround DSD Test";
            let mut body = Vec::new();
            body.extend_from_slice(b"TIT2");
            body.extend_from_slice(&(title_frame_data.len() as u32).to_be_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(title_frame_data);
            id3.extend_from_slice(b"ID3\x03\x00\x00");
            let sz = body.len() as u32;
            id3.push(((sz >> 21) & 0x7F) as u8);
            id3.push(((sz >> 14) & 0x7F) as u8);
            id3.push(((sz >> 7) & 0x7F) as u8);
            id3.push((sz & 0x7F) as u8);
            id3.extend_from_slice(&body);

            let total_size = id3_offset + id3.len() as u64;
            f.write_all(b"DSD ").unwrap();
            f.write_all(&28u64.to_le_bytes()).unwrap();
            f.write_all(&total_size.to_le_bytes()).unwrap();
            f.write_all(&id3_offset.to_le_bytes()).unwrap();

            f.write_all(b"fmt ").unwrap();
            f.write_all(&52u64.to_le_bytes()).unwrap();
            f.write_all(&1u32.to_le_bytes()).unwrap();
            f.write_all(&0u32.to_le_bytes()).unwrap();
            f.write_all(&7u32.to_le_bytes()).unwrap(); // 5.1
            f.write_all(&channels.to_le_bytes()).unwrap();
            f.write_all(&2_822_400u32.to_le_bytes()).unwrap();
            f.write_all(&1u32.to_le_bytes()).unwrap(); // LSB first
            f.write_all(&total_samples.to_le_bytes()).unwrap();
            f.write_all(&(block_size as u32).to_le_bytes()).unwrap();
            f.write_all(&0u32.to_le_bytes()).unwrap();

            f.write_all(b"data").unwrap();
            f.write_all(&(12 + data_payload_size).to_le_bytes()).unwrap();

            let tone = sigma_delta_sine(1000.0, 2_822_400, blocks * block_size, true);
            for b in 0..blocks {
                for c in 0..channels as usize {
                    if c == 2 {
                        // Put tone in Center channel only
                        f.write_all(&tone[b * block_size..(b + 1) * block_size])
                            .unwrap();
                    } else {
                        f.write_all(&vec![0x69u8; block_size]).unwrap();
                    }
                }
            }
            f.write_all(&id3).unwrap();
        }

        let info = probe_dsd_file(&dsf_path).unwrap();
        assert_eq!(info.container, "DSF");
        assert_eq!(info.channels, 6);
        assert_eq!(info.dsd_label, "DSD64");
        assert_eq!(info.metadata.title.as_deref(), Some("Surround DSD Test"));

        let mut reader = DsdStreamReader::open(&dsf_path, &DsdSettings::default()).unwrap();
        let chunk = reader.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.len(), 6);
        // Center channel (idx 2) should have signal while FL (idx 0) is near-silent
        let fl_peak = chunk[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        let fc_peak = chunk[2].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(fc_peak > 0.3, "Center peak too low: {}", fc_peak);
        assert!(fl_peak < 0.05, "FL leakage too high: {}", fl_peak);

        // Seek halfway and read again
        reader.seek(info.duration_secs * 0.5).unwrap();
        assert!(reader.next_chunk().unwrap().is_some());

        let _ = std::fs::remove_file(&dsf_path);
    }

    #[test]
    fn dff_dsd128_stereo_msb_round_trip() {
        let tmp = std::env::temp_dir();
        let dff_path = tmp.join("test_hires_player_dsd128.dff");
        let dsd_rate = 5_644_800u32; // DSD128
        let channels = 2u16;
        let bytes_per_ch = 16_384usize;
        let left = sigma_delta_sine(1000.0, dsd_rate, bytes_per_ch, false);
        let right = vec![0x69u8; bytes_per_ch];

        // Build valid DSDIFF (DFF) container
        let mut prop_body = Vec::new();
        prop_body.extend_from_slice(b"SND ");
        // FS chunk
        prop_body.extend_from_slice(b"FS  ");
        prop_body.extend_from_slice(&4u64.to_be_bytes());
        prop_body.extend_from_slice(&dsd_rate.to_be_bytes());
        // CHNL chunk
        prop_body.extend_from_slice(b"CHNL");
        prop_body.extend_from_slice(&(2 + 4 * channels as u64).to_be_bytes());
        prop_body.extend_from_slice(&channels.to_be_bytes());
        prop_body.extend_from_slice(b"SLFTSRGT");
        // CMPR chunk
        prop_body.extend_from_slice(b"CMPR");
        prop_body.extend_from_slice(&4u64.to_be_bytes());
        prop_body.extend_from_slice(b"DSD ");

        let mut dsd_payload = Vec::with_capacity(bytes_per_ch * 2);
        for i in 0..bytes_per_ch {
            dsd_payload.push(left[i]);
            dsd_payload.push(right[i]);
        }

        let mut frm8_body = Vec::new();
        frm8_body.extend_from_slice(b"DSD ");
        frm8_body.extend_from_slice(b"PROP");
        frm8_body.extend_from_slice(&(prop_body.len() as u64).to_be_bytes());
        frm8_body.extend_from_slice(&prop_body);
        frm8_body.extend_from_slice(b"DSD ");
        frm8_body.extend_from_slice(&(dsd_payload.len() as u64).to_be_bytes());
        frm8_body.extend_from_slice(&dsd_payload);

        {
            let mut f = File::create(&dff_path).unwrap();
            f.write_all(b"FRM8").unwrap();
            f.write_all(&(frm8_body.len() as u64).to_be_bytes()).unwrap();
            f.write_all(&frm8_body).unwrap();
        }

        let info = probe_dsd_file(&dff_path).unwrap();
        assert_eq!(info.container, "DFF");
        assert_eq!(info.dsd_label, "DSD128");
        assert_eq!(info.channels, 2);
        assert_eq!(info.channel_labels, vec!["FL", "FR"]);
        assert!(!info.lsb_first);

        let mut reader = DsdStreamReader::open(&dff_path, &DsdSettings::default()).unwrap();
        assert_eq!(reader.output_sample_rate, 176_400);
        let chunk = reader.next_chunk().unwrap().unwrap();
        let l_peak = chunk[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        let r_peak = chunk[1].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(l_peak > 0.3, "Left DSD128 tone too weak: {}", l_peak);
        assert!(r_peak < 0.05, "Right silence leakage too high: {}", r_peak);

        let _ = std::fs::remove_file(&dff_path);
    }
}
