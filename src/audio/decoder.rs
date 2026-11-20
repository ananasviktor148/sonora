use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use lofty::prelude::*;
use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::codecs::{CodecType, Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::audio::demo::{demo_spec_for_path, demo_track_info, DemoSynthDecoder};
use crate::audio::dsd::{
    default_channel_labels, is_dsd_extension, probe_dsd_file, DsdStreamReader,
};
use crate::config::DsdSettings;

/// Comprehensive list of supported audio & media extensions (Native Rust + DSD + FFmpeg Universal).
pub const SUPPORTED_AUDIO_EXTENSIONS: &[&str] = &[
    // Lossless & Hi-Res PCM
    "flac", "wav", "wave", "aiff", "aif", "aifc", "alac", "caf", "wv", "ape", "tta", "tak",
    "w64", "rf64", "au", "snd", "pcm", "raw",
    // DSD (1-bit High Resolution)
    "dsf", "dff",
    // Lossy & Modern Codecs
    "mp3", "mp2", "mp1", "aac", "m4a", "m4b", "m4p", "ogg", "oga", "opus", "wma", "mpc", "mp+",
    "mpp", "spx", "amr", "ac3", "eac3", "dts", "dtshd", "thd", "mlp", "ra", "rm", "mka",
    // Media containers containing audio tracks
    "mp4", "mkv", "webm", "mov", "avi", "ts", "m2ts", "wmv", "flv", "3gp",
];

pub fn is_supported_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let low = ext.to_ascii_lowercase();
            SUPPORTED_AUDIO_EXTENSIONS.iter().any(|&s| s == low)
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub sample_rate: u32,
    pub bit_depth: u16,
    pub channels: u16,
    pub channel_labels: Vec<String>,
    pub format: String,
    pub codec: String,
    pub is_dsd: bool,
    pub dsd_rate: Option<u32>,
    pub decoder_backend: String,
    pub album_art: Option<Vec<u8>>,
    pub file_path: PathBuf,
}

impl TrackInfo {
    pub fn is_hires(&self) -> bool {
        self.is_dsd || self.sample_rate > 48_000 || self.bit_depth >= 24
    }

    pub fn channel_layout_name(&self) -> String {
        channel_layout_label(self.channels)
    }
}

pub fn channel_layout_label(channels: u16) -> String {
    match channels {
        0 => "0ch".into(),
        1 => "Mono (1.0)".into(),
        2 => "Stereo (2.0)".into(),
        3 => "3.0".into(),
        4 => "Quad (4.0)".into(),
        5 => "5.0".into(),
        6 => "5.1 Surround".into(),
        7 => "6.1 Surround".into(),
        8 => "7.1 Surround".into(),
        n => format!("{}ch Multi", n),
    }
}

#[derive(Debug, Clone)]
pub struct StreamProperties {
    pub source_sample_rate: u32,
    pub decoded_sample_rate: u32,
    pub channels: u16,
    pub channel_labels: Vec<String>,
    pub bit_depth: u16,
    pub duration: f64,
    pub format_name: String,
    pub codec_name: String,
    pub backend: &'static str,
    pub is_dsd: bool,
    pub is_dop: bool,
    pub dsd_rate: Option<u32>,
}

/// Unified streaming audio decoder interface.
pub trait StreamingDecoder: Send {
    fn properties(&self) -> &StreamProperties;
    /// Decode next planar `f32` chunk (`channels × frames`). Returns `Ok(None)` at EOF.
    fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>>;
    /// Seek to target timestamp in seconds.
    fn seek(&mut self, seconds: f64) -> Result<()>;
}

/// Check once whether `ffmpeg` is available on PATH for universal codec fallback.
pub fn is_ffmpeg_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_window(&mut cmd);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
}

fn hide_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cmd;
    }
}

/// Open any supported music file using the best available streaming decoder:
/// 1. Native Rust DSD streaming decoder (`.dsf`, uncompressed `.dff`)
/// 2. Native Rust Symphonia streaming decoder (FLAC, WAV, ALAC, AIFF, CAF, MP3, AAC, M4A, OGG/Vorbis, MKV...)
/// 3. Universal FFmpeg streaming fallback (Opus, WavPack, APE, WMA, DTS, AC3, TrueHD, DST-DFF, video containers, etc.)
pub fn open_decoder(path: &Path, dsd_settings: &DsdSettings) -> Result<Box<dyn StreamingDecoder>> {
    // 0. Studio Reference Showcase track (`demo://...`)
    if demo_spec_for_path(path).is_some() {
        return Ok(Box::new(DemoSynthDecoder::open(path, dsd_settings)?));
    }

    // 1. Native DSD path
    if is_dsd_extension(path) {
        match DsdDecoderAdapter::open(path, dsd_settings) {
            Ok(dec) => return Ok(Box::new(dec)),
            Err(dsd_err) => {
                log::warn!(
                    "Native DSD decoder could not open {}: {} — trying FFmpeg fallback",
                    path.display(),
                    dsd_err
                );
                if is_ffmpeg_available() {
                    return Ok(Box::new(FfmpegStreamDecoder::open(path, 0.0)?));
                }
                return Err(dsd_err);
            }
        }
    }

    // 2. Native Symphonia path
    match SymphoniaStreamDecoder::open(path) {
        Ok(dec) => Ok(Box::new(dec)),
        Err(sym_err) => {
            log::info!(
                "Symphonia could not decode {} ({}), attempting FFmpeg universal decoder",
                path.display(),
                sym_err
            );
            // 3. Universal FFmpeg fallback
            if is_ffmpeg_available() {
                return Ok(Box::new(FfmpegStreamDecoder::open(path, 0.0)?));
            }
            Err(anyhow!(
                "Unsupported codec for native decoder ({}) and FFmpeg was not found in PATH",
                sym_err
            ))
        }
    }
}

// ── 1. Native DSD Streaming Adapter ──────────────────────────────────────────

struct DsdDecoderAdapter {
    reader: DsdStreamReader,
    props: StreamProperties,
}

impl DsdDecoderAdapter {
    fn open(path: &Path, settings: &DsdSettings) -> Result<Self> {
        let reader = DsdStreamReader::open(path, settings)?;
        let props = StreamProperties {
            source_sample_rate: reader.info.dsd_rate,
            decoded_sample_rate: reader.output_sample_rate,
            channels: reader.info.channels,
            channel_labels: reader.info.channel_labels.clone(),
            bit_depth: 1,
            duration: reader.info.duration_secs,
            format_name: reader.info.container.to_string(),
            codec_name: reader.info.dsd_label.clone(),
            backend: "Native DSD",
            is_dsd: true,
            is_dop: reader.is_dop,
            dsd_rate: Some(reader.info.dsd_rate),
        };
        Ok(Self { reader, props })
    }
}

impl StreamingDecoder for DsdDecoderAdapter {
    fn properties(&self) -> &StreamProperties {
        &self.props
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>> {
        self.reader.next_chunk()
    }

    fn seek(&mut self, seconds: f64) -> Result<()> {
        self.reader.seek(seconds)
    }
}

// ── 2. Native Symphonia Streaming Decoder ────────────────────────────────────

struct SymphoniaStreamDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    props: StreamProperties,
}

impl SymphoniaStreamDecoder {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )?;

        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("No supported audio track found"))?;

        let track_id = track.id;
        let cp = track.codec_params.clone();

        let decoder = symphonia::default::get_codecs().make(&cp, &DecoderOptions::default())?;

        let sample_rate = cp.sample_rate.unwrap_or(44_100);
        let (channels, channel_labels) = if let Some(ch_mask) = cp.channels {
            let labels = symphonia_channel_labels(ch_mask);
            (ch_mask.count() as u16, labels)
        } else {
            (2, default_channel_labels(2))
        };

        let bit_depth = cp
            .bits_per_sample
            .or(cp.bits_per_coded_sample)
            .unwrap_or(16) as u16;

        let duration = if let (Some(n_frames), Some(tb)) = (cp.n_frames, cp.time_base) {
            let t = tb.calc_time(n_frames);
            t.seconds as f64 + t.frac
        } else if let Some(n_frames) = cp.n_frames {
            n_frames as f64 / sample_rate as f64
        } else {
            0.0
        };

        let ext_upper = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("PCM")
            .to_uppercase();

        let props = StreamProperties {
            source_sample_rate: sample_rate,
            decoded_sample_rate: sample_rate,
            channels,
            channel_labels,
            bit_depth,
            duration,
            format_name: ext_upper,
            codec_name: codec_type_name(cp.codec).to_string(),
            backend: "Native Rust",
            is_dsd: false,
            is_dop: false,
            dsd_rate: None,
        };

        Ok(Self {
            format,
            decoder,
            track_id,
            props,
        })
    }
}

impl StreamingDecoder for SymphoniaStreamDecoder {
    fn properties(&self) -> &StreamProperties {
        &self.props
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(symphonia::core::errors::Error::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(_) => return Ok(None),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(symphonia::core::errors::Error::IoError(_)) => continue,
                Err(e) => return Err(e.into()),
            };

            let spec = *decoded.spec();
            let num_channels = spec.channels.count();
            let num_frames = decoded.frames();
            if num_frames == 0 || num_channels == 0 {
                continue;
            }

            let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
            sample_buf.copy_interleaved_ref(decoded);
            let interleaved = sample_buf.samples();

            let target_ch = self.props.channels as usize;
            let mut planar: Vec<Vec<f32>> = (0..target_ch)
                .map(|_| Vec::with_capacity(num_frames))
                .collect();

            for frame in 0..num_frames {
                let base = frame * num_channels;
                for ch in 0..target_ch {
                    let s = if ch < num_channels {
                        interleaved[base + ch]
                    } else {
                        0.0
                    };
                    planar[ch].push(s);
                }
            }

            return Ok(Some(planar));
        }
    }

    fn seek(&mut self, seconds: f64) -> Result<()> {
        let clamped = seconds.max(0.0);
        let secs = clamped.trunc() as u64;
        let frac = clamped.fract();
        let seek_to_acc = SeekTo::Time {
            time: Time::new(secs, frac),
            track_id: Some(self.track_id),
        };
        let seek_to_coarse = SeekTo::Time {
            time: Time::new(secs, frac),
            track_id: Some(self.track_id),
        };
        self.format
            .seek(SeekMode::Accurate, seek_to_acc)
            .or_else(|_| self.format.seek(SeekMode::Coarse, seek_to_coarse))?;
        self.decoder.reset();
        Ok(())
    }
}

fn symphonia_channel_labels(ch: Channels) -> Vec<String> {
    let mapping: &[(Channels, &str)] = &[
        (Channels::FRONT_LEFT, "FL"),
        (Channels::FRONT_RIGHT, "FR"),
        (Channels::FRONT_CENTRE, "FC"),
        (Channels::LFE1, "LFE"),
        (Channels::REAR_LEFT, "BL"),
        (Channels::REAR_RIGHT, "BR"),
        (Channels::FRONT_LEFT_CENTRE, "FLC"),
        (Channels::FRONT_RIGHT_CENTRE, "FRC"),
        (Channels::REAR_CENTRE, "BC"),
        (Channels::SIDE_LEFT, "SL"),
        (Channels::SIDE_RIGHT, "SR"),
        (Channels::TOP_CENTRE, "TC"),
        (Channels::TOP_FRONT_LEFT, "TFL"),
        (Channels::TOP_FRONT_CENTRE, "TFC"),
        (Channels::TOP_FRONT_RIGHT, "TFR"),
        (Channels::TOP_REAR_LEFT, "TBL"),
        (Channels::TOP_REAR_CENTRE, "TBC"),
        (Channels::TOP_REAR_RIGHT, "TBR"),
    ];

    let mut labels = Vec::new();
    for &(bit, name) in mapping {
        if ch.contains(bit) {
            labels.push(name.to_string());
        }
    }
    if labels.len() == ch.count() {
        labels
    } else {
        default_channel_labels(ch.count() as u16)
    }
}

fn codec_type_name(codec: CodecType) -> &'static str {
    use symphonia::core::codecs::*;
    match codec {
        CODEC_TYPE_FLAC => "FLAC",
        CODEC_TYPE_ALAC => "ALAC",
        CODEC_TYPE_MP3 => "MP3",
        CODEC_TYPE_MP2 => "MP2",
        CODEC_TYPE_MP1 => "MP1",
        CODEC_TYPE_AAC => "AAC",
        CODEC_TYPE_VORBIS => "Vorbis",
        CODEC_TYPE_OPUS => "Opus",
        CODEC_TYPE_PCM_S8 | CODEC_TYPE_PCM_U8 => "PCM 8-bit",
        CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S16BE | CODEC_TYPE_PCM_U16LE
        | CODEC_TYPE_PCM_U16BE => "PCM 16-bit",
        CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE | CODEC_TYPE_PCM_U24LE
        | CODEC_TYPE_PCM_U24BE => "PCM 24-bit",
        CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE | CODEC_TYPE_PCM_U32LE
        | CODEC_TYPE_PCM_U32BE => "PCM 32-bit",
        CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE => "PCM 32f",
        CODEC_TYPE_PCM_F64LE | CODEC_TYPE_PCM_F64BE => "PCM 64f",
        _ => "PCM / Audio",
    }
}

// ── 3. Universal FFmpeg Streaming Decoder ────────────────────────────────────

struct FfmpegStreamDecoder {
    path: PathBuf,
    props: StreamProperties,
    child: Option<Child>,
    stdout: Option<BufReader<ChildStdout>>,
    chunk_frames: usize,
}

impl FfmpegStreamDecoder {
    fn open(path: &Path, start_seconds: f64) -> Result<Self> {
        let probe = ffprobe_audio_stream(path)?;
        let mut dec = Self {
            path: path.to_path_buf(),
            props: probe,
            child: None,
            stdout: None,
            chunk_frames: 2048,
        };
        dec.spawn_at(start_seconds)?;
        Ok(dec)
    }

    fn spawn_at(&mut self, start_seconds: f64) -> Result<()> {
        self.terminate_child();

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-v").arg("error").arg("-nostdin");
        if start_seconds > 0.001 {
            cmd.arg("-ss").arg(format!("{:.4}", start_seconds));
        }
        cmd.arg("-i")
            .arg(&self.path)
            .arg("-vn")
            .arg("-sn")
            .arg("-dn")
            .arg("-f")
            .arg("f32le")
            .arg("-acodec")
            .arg("pcm_f32le")
            .arg("-ar")
            .arg(self.props.decoded_sample_rate.to_string())
            .arg("-ac")
            .arg(self.props.channels.to_string())
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        hide_window(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to start ffmpeg: {}", e))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to capture ffmpeg stdout"))?;

        self.child = Some(child);
        self.stdout = Some(BufReader::with_capacity(64 * 1024, stdout));
        Ok(())
    }

    fn terminate_child(&mut self) {
        self.stdout = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for FfmpegStreamDecoder {
    fn drop(&mut self) {
        self.terminate_child();
    }
}

impl StreamingDecoder for FfmpegStreamDecoder {
    fn properties(&self) -> &StreamProperties {
        &self.props
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>> {
        let stdout = match self.stdout.as_mut() {
            Some(s) => s,
            None => return Ok(None),
        };

        let ch = self.props.channels as usize;
        let bytes_per_frame = ch * 4;
        let mut raw = vec![0u8; self.chunk_frames * bytes_per_frame];

        let mut total = 0usize;
        while total < raw.len() {
            match stdout.read(&mut raw[total..]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        let frames = total / bytes_per_frame;
        if frames == 0 {
            return Ok(None);
        }

        let mut planar: Vec<Vec<f32>> = (0..ch).map(|_| Vec::with_capacity(frames)).collect();
        for f in 0..frames {
            let frame_off = f * bytes_per_frame;
            for (c, ch_buf) in planar.iter_mut().enumerate().take(ch) {
                let off = frame_off + c * 4;
                let sample =
                    f32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]]);
                ch_buf.push(sample);
            }
        }

        Ok(Some(planar))
    }

    fn seek(&mut self, seconds: f64) -> Result<()> {
        let clamped = seconds.clamp(0.0, self.props.duration.max(0.0));
        self.spawn_at(clamped)
    }
}

#[derive(Debug, Default)]
struct FfprobeMeta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    lyrics: Option<String>,
    sample_rate: u32,
    channels: u16,
    bit_depth: u16,
    duration: f64,
    codec_name: String,
    format_name: String,
}

fn run_ffprobe_json(path: &Path) -> Result<serde_json::Value> {
    let mut cmd = Command::new("ffprobe");
    cmd.args([
        "-v",
        "quiet",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-select_streams",
        "a:0",
    ])
    .arg(path)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    hide_window(&mut cmd);

    let output = cmd.output()?;
    if !output.status.success() {
        bail!("ffprobe failed for {}", path.display());
    }
    let val: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(val)
}

fn ffprobe_parse(path: &Path) -> Result<FfprobeMeta> {
    let v = run_ffprobe_json(path)?;
    let stream = v
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow!("No audio stream reported by ffprobe"))?;

    let sample_rate: u32 = stream
        .get("sample_rate")
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(48_000);

    let channels: u16 = stream
        .get("channels")
        .and_then(|x| x.as_u64())
        .unwrap_or(2) as u16;

    let bit_depth: u16 = stream
        .get("bits_per_raw_sample")
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
        .filter(|&b| b > 0)
        .or_else(|| {
            stream
                .get("bits_per_sample")
                .and_then(|x| x.as_u64())
                .map(|b| b as u16)
                .filter(|&b| b > 0)
        })
        .unwrap_or(24);

    let duration: f64 = stream
        .get("duration")
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            v.get("format")
                .and_then(|f| f.get("duration"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0.0);

    let codec_name = stream
        .get("codec_name")
        .and_then(|x| x.as_str())
        .unwrap_or("AUDIO")
        .to_uppercase();

    let format_name = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(&codec_name)
        .to_uppercase();

    let mut meta = FfprobeMeta {
        sample_rate,
        channels: channels.max(1),
        bit_depth,
        duration,
        codec_name,
        format_name,
        ..Default::default()
    };

    if let Some(tags) = v
        .get("format")
        .and_then(|f| f.get("tags"))
        .and_then(|t| t.as_object())
    {
        for (k, val) in tags {
            if let Some(s) = val.as_str() {
                let kl = k.to_ascii_lowercase();
                if kl == "title" && meta.title.is_none() {
                    meta.title = Some(s.to_string());
                } else if kl == "artist" && meta.artist.is_none() {
                    meta.artist = Some(s.to_string());
                } else if kl == "album" && meta.album.is_none() {
                    meta.album = Some(s.to_string());
                } else if kl.contains("lyrics") && meta.lyrics.is_none() {
                    meta.lyrics = Some(s.to_string());
                }
            }
        }
    }

    Ok(meta)
}

fn ffprobe_audio_stream(path: &Path) -> Result<StreamProperties> {
    let m = ffprobe_parse(path)?;
    // If FFmpeg decodes a DSD/DST stream, resample down to 352.8 kHz DXD PCM cleanly
    let (decoded_rate, is_dsd, dsd_rate) = if m.codec_name.contains("DSD") || m.codec_name == "DST"
    {
        let raw_rate = if m.sample_rate < 500_000 {
            m.sample_rate * 8
        } else {
            m.sample_rate
        };
        (176_400, true, Some(raw_rate))
    } else {
        (m.sample_rate, false, None)
    };

    Ok(StreamProperties {
        source_sample_rate: dsd_rate.unwrap_or(m.sample_rate),
        decoded_sample_rate: decoded_rate,
        channels: m.channels,
        channel_labels: default_channel_labels(m.channels),
        bit_depth: if is_dsd { 1 } else { m.bit_depth },
        duration: m.duration,
        format_name: m.format_name,
        codec_name: m.codec_name,
        backend: "FFmpeg Universal",
        is_dsd,
        is_dop: false,
        dsd_rate,
    })
}

// ── Fast Metadata Probing for Playlist & Now Playing ─────────────────────────

pub fn probe_track_info(path: &Path) -> TrackInfo {
    if let Some(spec) = demo_spec_for_path(path) {
        return demo_track_info(spec);
    }

    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Track")
        .to_string();

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("AUDIO")
        .to_uppercase();

    let mut info = TrackInfo {
        title: file_stem,
        artist: String::new(),
        album: String::new(),
        duration: 0.0,
        sample_rate: 0,
        bit_depth: 16,
        channels: 2,
        channel_labels: default_channel_labels(2),
        format: ext.clone(),
        codec: ext,
        is_dsd: false,
        dsd_rate: None,
        decoder_backend: "Native Rust".to_string(),
        album_art: None,
        file_path: path.to_path_buf(),
    };

    // Fast path for native DSD files (.dsf / .dff)
    if is_dsd_extension(path) {
        if let Ok(dsd) = probe_dsd_file(path) {
            info.is_dsd = true;
            info.dsd_rate = Some(dsd.dsd_rate);
            info.sample_rate = dsd.dsd_rate;
            info.bit_depth = 1;
            info.channels = dsd.channels;
            info.channel_labels = dsd.channel_labels;
            info.duration = dsd.duration_secs;
            info.format = dsd.container.to_string();
            info.codec = dsd.dsd_label;
            info.decoder_backend = "Native DSD".to_string();
            if let Some(t) = dsd.metadata.title {
                info.title = t;
            }
            if let Some(a) = dsd.metadata.artist {
                info.artist = a;
            }
            if let Some(al) = dsd.metadata.album {
                info.album = al;
            }
            if let Some(art) = dsd.metadata.album_art {
                info.album_art = Some(art);
            }
            return info;
        }
    }

    // Standard tag + property reading via lofty (supports FLAC, MP3, WAV, AIFF, OGG, Opus, M4A, APE, WavPack...)
    if let Ok(tagged) = lofty::read_from_path(path) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title() {
                if !t.is_empty() {
                    info.title = t.to_string();
                }
            }
            if let Some(a) = tag.artist() {
                if !a.is_empty() {
                    info.artist = a.to_string();
                }
            }
            if let Some(al) = tag.album() {
                if !al.is_empty() {
                    info.album = al.to_string();
                }
            }
            if let Some(pic) = tag.pictures().first() {
                info.album_art = Some(pic.data().to_vec());
            }
        }

        let props = tagged.properties();
        let dur = props.duration().as_secs_f64();
        if dur > 0.0 {
            info.duration = dur;
        }
        if let Some(sr) = props.sample_rate() {
            info.sample_rate = sr;
        }
        if let Some(ch) = props.channels() {
            info.channels = ch as u16;
            info.channel_labels = default_channel_labels(info.channels);
        }
        if let Some(bd) = props.bit_depth() {
            info.bit_depth = bd as u16;
        }
    }

    // If properties are still missing (e.g., MKV, WMA, DTS, AC3, DST-DFF), try ffprobe
    if (info.sample_rate == 0 || info.duration <= 0.0) && is_ffmpeg_available() {
        if let Ok(ff) = ffprobe_parse(path) {
            if info.sample_rate == 0 {
                info.sample_rate = ff.sample_rate;
            }
            if info.duration <= 0.0 {
                info.duration = ff.duration;
            }
            if ff.channels > 0 {
                info.channels = ff.channels;
                info.channel_labels = default_channel_labels(ff.channels);
            }
            info.bit_depth = ff.bit_depth;
            info.codec = ff.codec_name;
            if let Some(t) = ff.title {
                if info.title == path.file_stem().and_then(|s| s.to_str()).unwrap_or("") {
                    info.title = t;
                }
            }
            if info.artist.is_empty() {
                if let Some(a) = ff.artist {
                    info.artist = a;
                }
            }
            if info.album.is_empty() {
                if let Some(al) = ff.album {
                    info.album = al;
                }
            }
            info.decoder_backend = "FFmpeg Universal".to_string();
        }
    }

    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a minimal valid multi-channel 16-bit PCM WAV file for testing.
    fn write_test_wav(path: &Path, sample_rate: u32, channels: u16, frames: usize) {
        let bits_per_sample = 16u16;
        let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
        let block_align = channels * (bits_per_sample / 8);
        let data_size = (frames * block_align as usize) as u32;
        let riff_size = 36 + data_size;

        let mut f = File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&riff_size.to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_size.to_le_bytes()).unwrap();

        for i in 0..frames {
            let t = i as f32 / sample_rate as f32;
            for ch in 0..channels {
                let freq = 440.0 * (ch as f32 + 1.0);
                let sample = ((2.0 * std::f32::consts::PI * freq * t).sin() * 16000.0) as i16;
                f.write_all(&sample.to_le_bytes()).unwrap();
            }
        }
    }

    #[test]
    fn native_symphonia_decodes_multichannel_hires_wav_and_seeks() {
        let wav_path = std::env::temp_dir().join("test_hires_96k_6ch.wav");
        write_test_wav(&wav_path, 96_000, 6, 48_000); // 0.5 sec 96kHz 5.1ch

        let mut dec = open_decoder(&wav_path, &DsdSettings::default()).unwrap();
        let props = dec.properties().clone();
        assert_eq!(props.source_sample_rate, 96_000);
        assert_eq!(props.channels, 6);
        assert_eq!(props.backend, "Native Rust");

        let chunk = dec.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.len(), 6);
        assert!(!chunk[0].is_empty());

        dec.seek(0.25).unwrap();
        assert!(dec.next_chunk().unwrap().is_some());

        let _ = std::fs::remove_file(&wav_path);
    }

    #[test]
    fn universal_ffmpeg_fallback_decodes_opus_and_wavpack_if_available() {
        if !is_ffmpeg_available() {
            return;
        }
        let tmp = std::env::temp_dir();
        let wav = tmp.join("test_src_for_ff.wav");
        let opus = tmp.join("test_universal_decode.opus");
        let wv = tmp.join("test_universal_decode.wv");
        write_test_wav(&wav, 48_000, 2, 24_000);

        for (ext_path, codec) in [(&opus, "libopus"), (&wv, "wavpack")] {
            let mut cmd = Command::new("ffmpeg");
            cmd.args(["-y", "-v", "error", "-i"])
                .arg(&wav)
                .args(["-c:a", codec])
                .arg(ext_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            hide_window(&mut cmd);
            if cmd.status().map(|s| s.success()).unwrap_or(false) {
                let mut dec = open_decoder(ext_path, &DsdSettings::default()).unwrap();
                let props = dec.properties().clone();
                assert_eq!(props.channels, 2);
                let chunk = dec.next_chunk().unwrap().unwrap();
                assert_eq!(chunk.len(), 2);
                let peak = chunk[0].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                assert!(peak > 0.1, "Decoded audio silent for {:?}", ext_path);
            }
            let _ = std::fs::remove_file(ext_path);
        }

        let _ = std::fs::remove_file(&wav);
    }
}
