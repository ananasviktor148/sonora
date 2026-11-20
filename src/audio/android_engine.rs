#![allow(dead_code)]

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::{ImageBuffer, ImageFormat, Rgba};
use serde::Serialize;

use crate::audio::decoder::{
    is_supported_audio_file, open_decoder, probe_track_info, StreamProperties, StreamingDecoder,
    TrackInfo,
};
use crate::audio::demo::{
    demo_spec_for_path, demo_track_info, generate_swiss_cover_rgba, DEMO_CATALOG,
};
use crate::audio::dsp::{
    ChannelMuteSolo, MultiChannelRouter, SpectrumAnalyzer, StreamingUpsampler, TenBandEq,
    SPECTRUM_BANDS,
};
use crate::audio::loudness::{
    invalidate_cached_loudness, resolve_auto_volume, try_resolve_auto_volume_fast,
    AutoVolumeResult,
};
use crate::audio::ringbuf::{VisualizerTap, MAX_METER_CHANNELS, VISUALIZER_FFT_SIZE};
use crate::audio::{
    build_signal_path_str, db_to_lin_gain, output_soft_clip, AutoVolumeStatus, PlaybackState,
    SignalPath,
};
use crate::config::{AppConfig, AutoVolumeMode, DsdOutputMode, RepeatMode};
use crate::lyrics::{self, LyricsDocument};
use crate::playlists::{
    is_playlist_file, parse_m3u_file, Playlist, PlaylistEntry, PlaylistLibrary,
};

pub const VISUALIZER_FLOAT_LEN: usize = SPECTRUM_BANDS * 2 + MAX_METER_CHANNELS * 3; // 48 + 48 + 8 + 8 + 8 = 120

#[derive(Debug, Clone, Serialize)]
pub struct MobileTrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub sample_rate: u32,
    pub bit_depth: u16,
    pub channels: u16,
    pub channel_labels: Vec<String>,
    pub channel_layout: String,
    pub format: String,
    pub codec: String,
    pub is_dsd: bool,
    pub is_hires: bool,
    pub is_demo: bool,
    pub decoder_backend: String,
    pub file_path: String,
}

impl From<&TrackInfo> for MobileTrackInfo {
    fn from(t: &TrackInfo) -> Self {
        Self {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            duration: t.duration,
            sample_rate: t.sample_rate,
            bit_depth: t.bit_depth,
            channels: t.channels,
            channel_labels: t.channel_labels.clone(),
            channel_layout: t.channel_layout_name(),
            format: t.format.clone(),
            codec: t.codec.clone(),
            is_dsd: t.is_dsd,
            is_hires: t.is_hires(),
            is_demo: demo_spec_for_path(&t.file_path).is_some(),
            decoder_backend: t.decoder_backend.clone(),
            file_path: t.file_path.to_string_lossy().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MobileSnapshot {
    pub state: PlaybackState,
    pub position: f64,
    pub duration: f64,
    pub source_sample_rate: u32,
    pub output_sample_rate: u32,
    pub desired_output_rate: u32,
    pub source_channels: u16,
    pub output_channels: u16,
    pub upsampled: bool,
    pub downmixed: bool,
    pub muted: bool,
    pub track_info: Option<MobileTrackInfo>,
    pub auto_volume: AutoVolumeStatus,
    pub signal_path: SignalPath,
    pub config: AppConfig,
    pub channel_muted: Vec<bool>,
    pub channel_solo: Vec<bool>,
    pub active_playlist_id: String,
    pub playing_playlist_id: Option<String>,
    pub current_index: Option<usize>,
    pub playlists: Vec<Playlist>,
    pub lyrics: Option<LyricsDocument>,
    pub lyrics_offset_ms: i64,
    pub cover_revision: u64,
    pub error_message: Option<String>,
    pub toast: Option<String>,
}

struct PendingAutoVol {
    generation: u64,
    gain_db: f32,
    status: AutoVolumeStatus,
}

/// Pull-mode Hi-Res Audio, DSD, Upsampler, Multi-Channel, EQ, AudioGate Auto Volume,
/// Lyrics, and Playlist Engine designed to drive Android `AudioTrack` (`ENCODING_PCM_FLOAT`).
pub struct SonoraMobileEngine {
    pub config: AppConfig,
    pub library: PlaylistLibrary,
    pub playing_playlist_id: Option<String>,
    pub current_index: Option<usize>,

    decoder: Option<Box<dyn StreamingDecoder>>,
    props: Option<StreamProperties>,
    track_info: Option<TrackInfo>,
    upsampler: Option<StreamingUpsampler>,
    eq: TenBandEq,
    router: MultiChannelRouter,
    mute_solo: ChannelMuteSolo,

    visualizer: Arc<VisualizerTap>,
    spectrum: SpectrumAnalyzer,
    fft_scratch: [f32; VISUALIZER_FFT_SIZE],
    smooth_peaks: [f32; MAX_METER_CHANNELS],
    smooth_rms: [f32; MAX_METER_CHANNELS],
    peak_hold: [f32; MAX_METER_CHANNELS],

    state: PlaybackState,
    muted: bool,
    output_rate: u32,
    output_channels: u16,
    desired_output_rate: u32,
    seek_base_secs: f64,
    played_frames: u64,

    pending_interleaved: Vec<f32>,
    pending_offset: usize,

    auto_volume: AutoVolumeStatus,
    target_auto_gain_lin: f32,
    smooth_auto_gain_lin: f32,
    auto_vol_gen: Arc<Mutex<u64>>,
    pending_auto_vol: Arc<Mutex<Option<PendingAutoVol>>>,

    lyrics: Option<LyricsDocument>,
    lyrics_offset_ms: i64,
    cover_revision: u64,
    cached_cover_bytes: Option<Vec<u8>>,
    error_message: Option<String>,
    toast: Option<String>,
}

impl SonoraMobileEngine {
    pub fn new() -> Self {
        let mut config = AppConfig::load();
        // Android AudioTrack uses 32-bit float PCM (no raw DoP framing on standard AudioTrack)
        if config.dsd.output_mode == DsdOutputMode::DoP {
            config.dsd.output_mode = DsdOutputMode::Pcm176k;
        }
        let library = PlaylistLibrary::load();
        Self::from_parts(config, library)
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Self {
        Self::from_parts(AppConfig::default(), PlaylistLibrary::default_with_references())
    }

    fn from_parts(config: AppConfig, library: PlaylistLibrary) -> Self {
        let initial_pl_id = library.active_id.clone();
        let has_tracks = library
            .active()
            .map(|p| !p.tracks.is_empty())
            .unwrap_or(false);

        let eq = TenBandEq::new(48_000, 2, &config.eq);
        let mut engine = Self {
            config,
            library,
            playing_playlist_id: None,
            current_index: None,
            decoder: None,
            props: None,
            track_info: None,
            upsampler: None,
            eq,
            router: MultiChannelRouter::new(),
            mute_solo: ChannelMuteSolo::default(),
            visualizer: Arc::new(VisualizerTap::new()),
            spectrum: SpectrumAnalyzer::new(),
            fft_scratch: [0.0; VISUALIZER_FFT_SIZE],
            smooth_peaks: [0.0; MAX_METER_CHANNELS],
            smooth_rms: [0.0; MAX_METER_CHANNELS],
            peak_hold: [0.0; MAX_METER_CHANNELS],
            state: PlaybackState::Stopped,
            muted: false,
            output_rate: 48_000,
            output_channels: 2,
            desired_output_rate: 48_000,
            seek_base_secs: 0.0,
            played_frames: 0,
            pending_interleaved: Vec::new(),
            pending_offset: 0,
            auto_volume: AutoVolumeStatus::default(),
            target_auto_gain_lin: 1.0,
            smooth_auto_gain_lin: 1.0,
            auto_vol_gen: Arc::new(Mutex::new(0)),
            pending_auto_vol: Arc::new(Mutex::new(None)),
            lyrics: None,
            lyrics_offset_ms: 0,
            cover_revision: 0,
            cached_cover_bytes: None,
            error_message: None,
            toast: None,
        };

        if has_tracks {
            engine.load_track(&initial_pl_id, 0, false);
        }
        engine
    }

    pub fn take_toast(&mut self) -> Option<String> {
        self.toast.take()
    }

    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some(msg.into());
    }

    pub fn desired_output_rate(&self) -> u32 {
        self.desired_output_rate
    }

    pub fn current_position(&self) -> f64 {
        let dur = self.track_info.as_ref().map(|t| t.duration).unwrap_or(0.0);
        let raw = if self.output_rate > 0 {
            self.seek_base_secs + (self.played_frames as f64 / self.output_rate as f64)
        } else {
            self.seek_base_secs
        };
        if dur > 0.0 {
            raw.min(dur)
        } else {
            raw
        }
    }

    /// Notify Rust of the actual sample rate & channel count opened by Android `AudioTrack`.
    /// Rebuilds the 64-bit `rubato` Sinc resampler cleanly whenever the rate changes.
    pub fn set_actual_output_format(&mut self, rate: u32, channels: u16) {
        let rate = rate.clamp(8_000, 384_000);
        let channels = channels.clamp(1, 8);
        if self.output_rate == rate && self.output_channels == channels {
            return;
        }
        let cur_pos = self.current_position();
        self.seek_base_secs = cur_pos;
        self.played_frames = 0;
        self.output_rate = rate;
        self.output_channels = channels;
        self.pending_interleaved.clear();
        self.pending_offset = 0;
        self.rebuild_dsp_chain();
    }

    fn rebuild_dsp_chain(&mut self) {
        let Some(ref props) = self.props else {
            self.upsampler = None;
            return;
        };
        let src_rate = props.decoded_sample_rate.max(8_000);
        let src_ch = props.channels.max(1) as usize;
        self.upsampler = if self.output_rate != src_rate {
            StreamingUpsampler::new(
                src_rate,
                self.output_rate,
                src_ch,
                self.config.resampler_quality,
                self.config.filter_rolloff,
            )
            .ok()
        } else {
            None
        };
        self.eq = TenBandEq::new(self.output_rate, src_ch, &self.config.eq);
        self.router.reset();
    }

    fn compute_desired_rate(props: &StreamProperties, config: &AppConfig) -> u32 {
        let resolved = config.upsample_mode.resolve_rate(props.decoded_sample_rate);
        // Android AudioTrack reliably supports up to 192 kHz float PCM on modern phones & USB DACs
        resolved.clamp(44_100, 192_000)
    }

    fn poll_pending_auto_volume(&mut self) {
        let maybe = {
            let mut guard = self.pending_auto_vol.lock().unwrap();
            guard.take()
        };
        if let Some(p) = maybe {
            let cur_gen = *self.auto_vol_gen.lock().unwrap();
            if p.generation == cur_gen {
                self.auto_volume = p.status;
                self.target_auto_gain_lin = if self.auto_volume.active
                    && self.auto_volume.mode != AutoVolumeMode::Off
                {
                    db_to_lin_gain(p.gain_db)
                } else {
                    1.0
                };
            }
        }
    }

    /// Real-time pull callback invoked by Android's high-priority `AudioTrack` thread.
    /// Returns:
    /// - `> 0`: number of interleaved `f32` samples written into `out`
    /// - `0`: paused / stopped (caller should sleep briefly)
    /// - `-1`: track reached EOF cleanly (caller should advance playlist)
    pub fn render_audio(&mut self, out: &mut [f32]) -> i32 {
        self.poll_pending_auto_volume();

        if self.state != PlaybackState::Playing || out.is_empty() {
            return 0;
        }

        let channels = (self.output_channels as usize).max(1);
        let sample_rate = self.output_rate.max(8_000) as f32;
        let ramp_coeff = (1.0 - (-1.0 / (sample_rate * 0.045)).exp()).clamp(0.0001, 1.0);
        let user_gain = if self.muted {
            0.0
        } else {
            self.config.volume.clamp(0.0, 1.0)
        };
        let target_auto = self.target_auto_gain_lin;
        let soft_clip = self.config.soft_clip;

        let mut written = 0usize;
        while written < out.len() {
            if self.pending_offset < self.pending_interleaved.len() {
                let avail = self.pending_interleaved.len() - self.pending_offset;
                let take = avail.min(out.len() - written);
                // Align to channel frames when applying smooth per-frame gain
                let take_aligned = if take >= channels {
                    (take / channels) * channels
                } else {
                    take
                };
                let frames = take_aligned / channels;
                for f in 0..frames {
                    self.smooth_auto_gain_lin +=
                        (target_auto - self.smooth_auto_gain_lin) * ramp_coeff;
                    let total_gain = user_gain * self.smooth_auto_gain_lin;
                    let dst_base = written + f * channels;
                    let src_base = self.pending_offset + f * channels;
                    for c in 0..channels {
                        out[dst_base + c] = output_soft_clip(
                            self.pending_interleaved[src_base + c] * total_gain,
                            soft_clip,
                        );
                    }
                }
                for i in (frames * channels)..take_aligned {
                    out[written + i] = output_soft_clip(
                        self.pending_interleaved[self.pending_offset + i]
                            * user_gain
                            * self.smooth_auto_gain_lin,
                        soft_clip,
                    );
                }
                self.pending_offset += take_aligned;
                written += take_aligned;
                if self.pending_offset >= self.pending_interleaved.len() {
                    self.pending_interleaved.clear();
                    self.pending_offset = 0;
                }
                continue;
            }

            // Decode next streaming chunk
            let next = match self.decoder.as_mut() {
                Some(dec) => dec.next_chunk(),
                None => break,
            };

            match next {
                Ok(Some(mut planar)) => {
                    MultiChannelRouter::apply_mute_solo(&mut planar, &self.mute_solo);
                    self.visualizer.push_planar(&planar);

                    let mut processed = if let Some(ref mut up) = self.upsampler {
                        match up.process(&planar) {
                            Ok(r) => r,
                            Err(e) => {
                                log::warn!("Upsampler error: {}", e);
                                planar
                            }
                        }
                    } else {
                        planar
                    };

                    if processed.first().map(|c| c.is_empty()).unwrap_or(true) {
                        continue;
                    }

                    self.eq
                        .update(self.output_rate, processed.len(), &self.config.eq);
                    self.eq.process_in_place(&mut processed, &self.config.eq);

                    self.pending_interleaved = self.router.mix_to_interleaved(
                        &processed,
                        channels,
                        &self.config.multichannel,
                        self.config.headroom_db,
                        self.config.soft_clip,
                        false,
                    );
                    self.pending_offset = 0;
                }
                Ok(None) => {
                    // Flush resampler tail on clean EOF
                    if let Some(ref mut up) = self.upsampler {
                        if let Ok(mut tail) = up.flush() {
                            if tail.first().map(|c| !c.is_empty()).unwrap_or(false) {
                                self.eq.process_in_place(&mut tail, &self.config.eq);
                                self.pending_interleaved = self.router.mix_to_interleaved(
                                    &tail,
                                    channels,
                                    &self.config.multichannel,
                                    self.config.headroom_db,
                                    self.config.soft_clip,
                                    false,
                                );
                                self.pending_offset = 0;
                            }
                        }
                    }
                    self.decoder = None;
                    if self.pending_interleaved.is_empty() {
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Decode error: {}", e);
                    self.error_message = Some(format!("Decode error: {}", e));
                    self.decoder = None;
                    break;
                }
            }
        }

        if written > 0 {
            self.played_frames += (written / channels) as u64;
            written as i32
        } else if self.decoder.is_none() && self.pending_offset >= self.pending_interleaved.len() {
            self.state = PlaybackState::Finished;
            self.visualizer.reset_meters();
            -1
        } else {
            0
        }
    }

    pub fn load_track(&mut self, playlist_id: &str, index: usize, autoplay: bool) {
        let Some(entry) = self
            .library
            .get(playlist_id)
            .and_then(|pl| pl.tracks.get(index))
            .cloned()
        else {
            return;
        };

        self.playing_playlist_id = Some(playlist_id.to_string());
        self.current_index = Some(index);
        self.mute_solo = ChannelMuteSolo::default();
        self.lyrics_offset_ms = 0;
        self.error_message = None;
        self.pending_interleaved.clear();
        self.pending_offset = 0;
        self.seek_base_secs = 0.0;
        self.played_frames = 0;
        self.visualizer.reset_meters();

        // Android AudioTrack cannot carry raw DoP markers, so ensure PCM decimation for DSD
        let mut dsd_cfg = self.config.dsd.clone();
        if dsd_cfg.output_mode == DsdOutputMode::DoP {
            dsd_cfg.output_mode = DsdOutputMode::Pcm176k;
        }

        let mut info = probe_track_info(&entry.path);
        match open_decoder(&entry.path, &dsd_cfg) {
            Ok(dec) => {
                let props = dec.properties().clone();
                info.sample_rate = props.source_sample_rate;
                info.bit_depth = props.bit_depth;
                info.channels = props.channels;
                info.channel_labels = props.channel_labels.clone();
                info.format = props.format_name.clone();
                info.codec = props.codec_name.clone();
                info.is_dsd = props.is_dsd;
                info.dsd_rate = props.dsd_rate;
                info.decoder_backend = props.backend.to_string();
                if props.duration > 0.0 {
                    info.duration = props.duration;
                }

                self.desired_output_rate = Self::compute_desired_rate(&props, &self.config);
                self.output_rate = self.desired_output_rate;
                self.output_channels = 2;

                self.cached_cover_bytes = Self::build_cover_bytes(&info);
                self.cover_revision = self.cover_revision.wrapping_add(1);

                self.decoder = Some(dec);
                self.props = Some(props);
                self.track_info = Some(info);
                self.rebuild_dsp_chain();
                self.state = if autoplay {
                    PlaybackState::Playing
                } else {
                    PlaybackState::Paused
                };

                self.lyrics = lyrics::load_local_lyrics(&entry.path);
                self.trigger_auto_volume(Some(entry.path.clone()), false);
            }
            Err(e) => {
                log::error!("Failed to open {}: {}", entry.path.display(), e);
                self.decoder = None;
                self.props = None;
                self.state = PlaybackState::Stopped;
                self.error_message = Some(format!("Cannot decode track: {}", e));
            }
        }
    }

    fn build_cover_bytes(info: &TrackInfo) -> Option<Vec<u8>> {
        if let Some(ref bytes) = info.album_art {
            return Some(bytes.clone());
        }
        if let Some(spec) = demo_spec_for_path(&info.file_path) {
            let (w, h, rgba) = generate_swiss_cover_rgba(spec.cover_variant);
            if let Some(img) = ImageBuffer::<Rgba<u8>, _>::from_raw(w as u32, h as u32, rgba) {
                let mut png = Vec::new();
                if img
                    .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                    .is_ok()
                {
                    return Some(png);
                }
            }
        }
        None
    }

    pub fn cover_art_bytes(&self) -> Option<&[u8]> {
        self.cached_cover_bytes.as_deref()
    }

    pub fn play(&mut self) {
        if self.decoder.is_some() {
            self.state = PlaybackState::Playing;
            return;
        }
        let pid = self.playback_playlist_id();
        let len = self.library.get(&pid).map(|p| p.tracks.len()).unwrap_or(0);
        if let Some(idx) = self.current_index {
            if idx < len {
                self.load_track(&pid, idx, true);
                return;
            }
        }
        if len > 0 {
            self.load_track(&pid, 0, true);
        }
    }

    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
            self.visualizer.reset_meters();
        }
    }

    pub fn toggle_play_pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn stop(&mut self) {
        self.state = PlaybackState::Stopped;
        self.decoder = None;
        self.pending_interleaved.clear();
        self.pending_offset = 0;
        self.seek_base_secs = 0.0;
        self.played_frames = 0;
        self.visualizer.reset_meters();
    }

    pub fn seek(&mut self, target_secs: f64) {
        let Some(ref info) = self.track_info else {
            return;
        };
        let path = info.file_path.clone();
        let dur = info.duration.max(0.0);
        let clamped = target_secs.clamp(0.0, dur);

        self.pending_interleaved.clear();
        self.pending_offset = 0;

        let mut dsd_cfg = self.config.dsd.clone();
        if dsd_cfg.output_mode == DsdOutputMode::DoP {
            dsd_cfg.output_mode = DsdOutputMode::Pcm176k;
        }

        if self.decoder.is_none() {
            self.decoder = open_decoder(&path, &dsd_cfg).ok();
        }
        if let Some(ref mut dec) = self.decoder {
            if dec.seek(clamped).is_err() {
                if let Ok(mut fresh) = open_decoder(&path, &dsd_cfg) {
                    let _ = fresh.seek(clamped);
                    *dec = fresh;
                }
            }
        }
        if let Some(ref mut up) = self.upsampler {
            up.reset();
        }
        self.eq.reset();
        self.router.reset();
        self.seek_base_secs = clamped;
        self.played_frames = 0;
    }

    pub fn playback_playlist_id(&self) -> String {
        if let Some(ref pid) = self.playing_playlist_id {
            if self.library.get(pid).is_some() {
                return pid.clone();
            }
        }
        self.library.active_id.clone()
    }

    pub fn next_track(&mut self, user_triggered: bool) {
        let pid = self.playback_playlist_id();
        let len = self.library.get(&pid).map(|p| p.tracks.len()).unwrap_or(0);
        if len == 0 {
            return;
        }
        if !user_triggered && self.config.repeat_mode == RepeatMode::One {
            if let Some(idx) = self.current_index {
                self.load_track(&pid, idx.min(len - 1), true);
                return;
            }
        }
        if self.config.shuffle && len > 1 {
            let now_nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(1);
            let mut next = now_nanos % len;
            if Some(next) == self.current_index {
                next = (next + 1) % len;
            }
            self.load_track(&pid, next, true);
            return;
        }
        match self.current_index {
            Some(idx) if idx + 1 < len => self.load_track(&pid, idx + 1, true),
            Some(_) => {
                if user_triggered || self.config.repeat_mode == RepeatMode::All {
                    self.load_track(&pid, 0, true);
                } else {
                    self.state = PlaybackState::Stopped;
                }
            }
            None => self.load_track(&pid, 0, true),
        }
    }

    pub fn prev_track(&mut self) {
        let pid = self.playback_playlist_id();
        let len = self.library.get(&pid).map(|p| p.tracks.len()).unwrap_or(0);
        if len == 0 {
            return;
        }
        if self.current_position() > 3.0 && self.state == PlaybackState::Playing {
            self.seek(0.0);
            return;
        }
        let prev = match self.current_index {
            Some(idx) if idx > 0 => idx - 1,
            _ => len - 1,
        };
        self.load_track(&pid, prev, true);
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.config.volume = vol.clamp(0.0, 1.0);
        self.muted = false;
        self.config.save();
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn toggle_shuffle(&mut self) {
        self.config.shuffle = !self.config.shuffle;
        self.config.save();
    }

    pub fn cycle_repeat(&mut self) {
        self.config.repeat_mode = match self.config.repeat_mode {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        self.config.save();
    }

    pub fn update_config(&mut self, mut new_cfg: AppConfig) {
        if new_cfg.dsd.output_mode == DsdOutputMode::DoP {
            new_cfg.dsd.output_mode = DsdOutputMode::Pcm176k;
        }
        let dsd_changed = self.config.dsd != new_cfg.dsd;
        let up_changed = self.config.upsample_mode != new_cfg.upsample_mode
            || self.config.resampler_quality != new_cfg.resampler_quality
            || self.config.filter_rolloff != new_cfg.filter_rolloff;
        let av_changed = self.config.auto_volume != new_cfg.auto_volume
            || self.config.dsd.gain_boost_6db != new_cfg.dsd.gain_boost_6db;

        self.config = new_cfg;
        self.config.save();

        if dsd_changed {
            // Reopen decoder at current position so new DSD decimator LPF / rate / +6dB engages seamlessly
            if let Some(ref info) = self.track_info {
                let path = info.file_path.clone();
                let pos = self.current_position();
                if let Ok(mut dec) = open_decoder(&path, &self.config.dsd) {
                    if pos > 0.05 {
                        let _ = dec.seek(pos);
                    }
                    let props = dec.properties().clone();
                    self.desired_output_rate = Self::compute_desired_rate(&props, &self.config);
                    self.decoder = Some(dec);
                    self.props = Some(props);
                    self.seek_base_secs = pos;
                    self.played_frames = 0;
                    self.pending_interleaved.clear();
                    self.pending_offset = 0;
                    self.rebuild_dsp_chain();
                }
            }
        } else if up_changed {
            if let Some(ref props) = self.props {
                self.desired_output_rate = Self::compute_desired_rate(props, &self.config);
                self.rebuild_dsp_chain();
            }
        }

        if av_changed {
            self.trigger_auto_volume(None, false);
        }
    }

    pub fn set_channel_mute_solo(&mut self, channel: usize, mute: bool, solo: bool) {
        if channel < MAX_METER_CHANNELS {
            self.mute_solo.muted[channel] = mute;
            self.mute_solo.solo[channel] = solo;
        }
    }

    pub fn reset_mute_solo(&mut self) {
        self.mute_solo = ChannelMuteSolo::default();
    }

    // ── Auto Adjust Playback Volume (AudioGate) ──────────────────────────────

    pub fn cycle_auto_volume_mode(&mut self) {
        self.config.auto_volume.mode = match self.config.auto_volume.mode {
            AutoVolumeMode::Off => AutoVolumeMode::Song,
            AutoVolumeMode::Song => AutoVolumeMode::Album,
            AutoVolumeMode::Album => AutoVolumeMode::Off,
        };
        self.config.save();
        self.trigger_auto_volume(None, false);
        self.set_toast(format!(
            "AUTO VOLUME · {}",
            self.config.auto_volume.mode.label().to_uppercase()
        ));
    }

    pub fn reanalyze_auto_volume(&mut self) {
        self.trigger_auto_volume(None, true);
        self.set_toast("RE-ANALYZING LOUDNESS (ITU-R BS.1770-4)");
    }

    fn current_track_path(&self) -> Option<PathBuf> {
        self.track_info.as_ref().map(|t| t.file_path.clone())
    }

    fn collect_album_peer_paths(&self, track_path: &Path) -> Vec<PathBuf> {
        let pid = self.playback_playlist_id();
        let Some(pl) = self.library.get(&pid) else {
            return vec![track_path.to_path_buf()];
        };
        let entry_album = pl
            .tracks
            .iter()
            .find(|t| t.path == track_path)
            .map(|t| t.album.trim().to_string())
            .filter(|a| !a.is_empty())
            .or_else(|| {
                self.track_info
                    .as_ref()
                    .filter(|info| info.file_path == track_path)
                    .map(|info| info.album.trim().to_string())
                    .filter(|a| !a.is_empty())
            });

        let mut peers = Vec::new();
        if let Some(ref album_name) = entry_album {
            let needle = album_name.to_ascii_lowercase();
            for t in &pl.tracks {
                if t.album.trim().to_ascii_lowercase() == needle {
                    peers.push(t.path.clone());
                }
            }
        } else if demo_spec_for_path(track_path).is_none() {
            if let Some(parent) = track_path.parent() {
                for t in &pl.tracks {
                    if !t.is_demo && t.path.parent() == Some(parent) {
                        peers.push(t.path.clone());
                    }
                }
            }
        }
        if !peers.iter().any(|p| p == track_path) {
            peers.insert(0, track_path.to_path_buf());
        }
        peers
    }

    fn apply_av_result(&mut self, res: AutoVolumeResult) {
        let active = res.mode != AutoVolumeMode::Off;
        self.auto_volume = AutoVolumeStatus {
            active,
            analyzing: false,
            mode: res.mode,
            applied_gain_db: if active { res.gain_db } else { 0.0 },
            measured_lufs: if active { Some(res.measured_lufs) } else { None },
            peak_dbfs: res.peak_dbfs,
            peak_limited: res.peak_limited,
            bypassed_for_dop: false,
            source_label: res.source_label,
        };
        self.target_auto_gain_lin = if active {
            db_to_lin_gain(res.gain_db)
        } else {
            1.0
        };
        if self.state != PlaybackState::Playing {
            self.smooth_auto_gain_lin = self.target_auto_gain_lin;
        }
    }

    pub fn trigger_auto_volume(&mut self, explicit_path: Option<PathBuf>, force_rescan: bool) {
        let gen = {
            let mut g = self.auto_vol_gen.lock().unwrap();
            *g += 1;
            *g
        };

        let av_cfg = self.config.auto_volume.clone();
        if av_cfg.mode == AutoVolumeMode::Off {
            self.auto_volume = AutoVolumeStatus::default();
            self.target_auto_gain_lin = 1.0;
            return;
        }

        let Some(track_path) = explicit_path.or_else(|| self.current_track_path()) else {
            return;
        };

        let album_peers = if av_cfg.mode == AutoVolumeMode::Album {
            self.collect_album_peer_paths(&track_path)
        } else {
            vec![track_path.clone()]
        };

        if force_rescan {
            invalidate_cached_loudness(&album_peers);
        } else if let Some(fast_res) =
            try_resolve_auto_volume_fast(&track_path, &album_peers, &av_cfg, &self.config.dsd)
        {
            self.apply_av_result(fast_res);
            return;
        }

        self.auto_volume = AutoVolumeStatus {
            active: false,
            analyzing: true,
            mode: av_cfg.mode,
            applied_gain_db: 0.0,
            measured_lufs: None,
            peak_dbfs: None,
            peak_limited: false,
            bypassed_for_dop: false,
            source_label: "Analyzing…".to_string(),
        };
        self.target_auto_gain_lin = 1.0;

        let gen_arc = Arc::clone(&self.auto_vol_gen);
        let slot = Arc::clone(&self.pending_auto_vol);
        let dsd_cfg = self.config.dsd.clone();

        std::thread::spawn(move || {
            let res = resolve_auto_volume(&track_path, &album_peers, &av_cfg, &dsd_cfg, force_rescan);
            if *gen_arc.lock().unwrap() != gen {
                return;
            }
            let pending = match res {
                Ok(r) => {
                    let active = r.mode != AutoVolumeMode::Off;
                    PendingAutoVol {
                        generation: gen,
                        gain_db: if active { r.gain_db } else { 0.0 },
                        status: AutoVolumeStatus {
                            active,
                            analyzing: false,
                            mode: r.mode,
                            applied_gain_db: if active { r.gain_db } else { 0.0 },
                            measured_lufs: if active { Some(r.measured_lufs) } else { None },
                            peak_dbfs: r.peak_dbfs,
                            peak_limited: r.peak_limited,
                            bypassed_for_dop: false,
                            source_label: r.source_label,
                        },
                    }
                }
                Err(_) => PendingAutoVol {
                    generation: gen,
                    gain_db: 0.0,
                    status: AutoVolumeStatus {
                        active: false,
                        analyzing: false,
                        mode: av_cfg.mode,
                        applied_gain_db: 0.0,
                        measured_lufs: None,
                        peak_dbfs: None,
                        peak_limited: false,
                        bypassed_for_dop: false,
                        source_label: "Scan Unavailable".to_string(),
                    },
                },
            };
            *slot.lock().unwrap() = Some(pending);
        });
    }

    // ── Playlists & Library Management ───────────────────────────────────────

    pub fn import_paths(&mut self, paths: Vec<PathBuf>) -> usize {
        let target_pl_id = self.library.active_id.clone();
        let has_audio = paths.iter().any(|p| is_supported_audio_file(p));
        let replace_demo = has_audio
            && target_pl_id == "pl-studio-reference"
            && self
                .library
                .active()
                .map(|p| !p.tracks.is_empty() && p.tracks.iter().all(|t| t.is_demo))
                .unwrap_or(false);
        if replace_demo {
            if let Some(pl) = self.library.active_mut() {
                pl.tracks.clear();
            }
            if self.playing_playlist_id.as_deref() == Some(target_pl_id.as_str()) {
                self.current_index = None;
            }
        }

        let mut added = 0usize;
        let mut first_new: Option<usize> = None;

        for p in paths {
            if is_playlist_file(&p) {
                if let Ok((name, entries)) = parse_m3u_file(&p) {
                    let cnt = entries.len();
                    let id = self.library.create_playlist(&name, entries);
                    self.library.save();
                    self.set_toast(format!("IMPORTED {} ({} TRACKS)", name.to_uppercase(), cnt));
                    if cnt > 0 && self.state != PlaybackState::Playing {
                        self.load_track(&id, 0, false);
                    }
                }
                continue;
            }
            if is_supported_audio_file(&p) {
                let info = probe_track_info(&p);
                if let Some(pl) = self.library.get_mut(&target_pl_id) {
                    if first_new.is_none() {
                        first_new = Some(pl.tracks.len());
                    }
                    pl.tracks.push(PlaylistEntry::from_info(info));
                    added += 1;
                }
            }
        }

        if added > 0 {
            self.library.save();
            self.set_toast(format!(
                "ADDED {} TRACK{}",
                added,
                if added == 1 { "" } else { "S" }
            ));
            if self.current_index.is_none() && self.state != PlaybackState::Playing {
                if let Some(idx) = first_new {
                    self.load_track(&target_pl_id, idx, true);
                }
            }
        }
        added
    }

    pub fn create_playlist(&mut self, name: &str, copy_current: bool, seed_index: Option<usize>) {
        let tracks = if let Some(idx) = seed_index {
            self.library
                .active()
                .and_then(|p| p.tracks.get(idx))
                .cloned()
                .map(|t| vec![t])
                .unwrap_or_default()
        } else if copy_current {
            self.library
                .active()
                .map(|p| p.tracks.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let id = self.library.create_playlist(name, tracks);
        let final_name = self
            .library
            .get(&id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| name.to_string());
        self.library.save();
        self.set_toast(format!("CREATED PLAYLIST · {}", final_name.to_uppercase()));
    }

    pub fn switch_playlist(&mut self, id: &str) {
        if self.library.get(id).is_some() {
            self.library.active_id = id.to_string();
            self.library.save();
        }
    }

    pub fn rename_playlist(&mut self, id: &str, new_name: &str) {
        if self.library.rename_playlist(id, new_name) {
            self.library.save();
            self.set_toast(format!("RENAMED · {}", new_name.trim().to_uppercase()));
        }
    }

    pub fn duplicate_playlist(&mut self, id: &str) {
        if let Some(dup_id) = self.library.duplicate_playlist(id) {
            self.library.save();
            if let Some(pl) = self.library.get(&dup_id) {
                self.set_toast(format!("DUPLICATED · {}", pl.name.to_uppercase()));
            }
        }
    }

    pub fn delete_playlist(&mut self, id: &str) {
        let name = self
            .library
            .get(id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        if self.library.delete_playlist(id) {
            if self.playing_playlist_id.as_deref() == Some(id) {
                self.playing_playlist_id = None;
                self.current_index = None;
            }
            self.library.save();
            self.set_toast(format!("DELETED · {}", name.to_uppercase()));
        }
    }

    pub fn add_track_to_playlist(&mut self, target_id: &str, source_index: usize) {
        let Some(entry) = self
            .library
            .active()
            .and_then(|p| p.tracks.get(source_index))
            .cloned()
        else {
            return;
        };
        if self.library.add_track_to(target_id, entry.clone()) {
            self.library.save();
            let pname = self
                .library
                .get(target_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "PLAYLIST".into());
            self.set_toast(format!("ADDED TO {}", pname.to_uppercase()));
        }
    }

    pub fn remove_track(&mut self, index: usize) {
        let active_id = self.library.active_id.clone();
        let is_playing_pl = self.playing_playlist_id.as_deref() == Some(active_id.as_str());
        if let Some(pl) = self.library.active_mut() {
            if index < pl.tracks.len() {
                pl.tracks.remove(index);
                if is_playing_pl {
                    if let Some(cur) = self.current_index {
                        if index == cur {
                            self.current_index = None;
                        } else if index < cur {
                            self.current_index = Some(cur - 1);
                        }
                    }
                }
                self.library.save();
            }
        }
    }

    pub fn move_track(&mut self, from: usize, to: usize) {
        let active_id = self.library.active_id.clone();
        let is_playing_pl = self.playing_playlist_id.as_deref() == Some(active_id.as_str());
        if let Some(pl) = self.library.active_mut() {
            if from < pl.tracks.len() && to < pl.tracks.len() && from != to {
                pl.tracks.swap(from, to);
                if is_playing_pl {
                    if self.current_index == Some(from) {
                        self.current_index = Some(to);
                    } else if self.current_index == Some(to) {
                        self.current_index = Some(from);
                    }
                }
                self.library.save();
            }
        }
    }

    pub fn clear_active_playlist(&mut self) {
        let active_id = self.library.active_id.clone();
        let is_playing_pl = self.playing_playlist_id.as_deref() == Some(active_id.as_str());
        if let Some(pl) = self.library.active_mut() {
            pl.tracks.clear();
        }
        if is_playing_pl {
            self.current_index = None;
            self.stop();
        }
        self.library.save();
    }

    pub fn restore_studio_references(&mut self) {
        let demos: Vec<PlaylistEntry> = DEMO_CATALOG
            .iter()
            .map(|s| PlaylistEntry::from_info(demo_track_info(s)))
            .collect();
        let active_id = self.library.active_id.clone();
        if let Some(pl) = self.library.active_mut() {
            pl.tracks.extend(demos);
        }
        self.library.save();
        self.set_toast("ADDED STUDIO REFERENCE TRACKS");
        if self.current_index.is_none() {
            self.load_track(&active_id, 0, false);
        }
    }

    // ── Lyrics Controls ──────────────────────────────────────────────────────

    pub fn upload_lyrics_text(&mut self, raw_text: &str, source_label: &str) {
        let doc = lyrics::parse_lyrics(raw_text, source_label);
        if let Some(path) = self.current_track_path() {
            let _ = lyrics::save_to_lyrics_store(&path, &doc.raw_text);
        }
        self.lyrics_offset_ms = 0;
        let kind = if doc.is_synced() {
            format!("{} SYNCED LINES", doc.synced_lines.len())
        } else {
            format!("{} LINES", doc.plain_lines.len())
        };
        self.lyrics = Some(doc);
        self.set_toast(format!("LYRICS LOADED · {}", kind));
    }

    pub fn set_lyrics_offset_ms(&mut self, offset_ms: i64) {
        self.lyrics_offset_ms = offset_ms.clamp(-10_000, 10_000);
    }

    pub fn get_lyrics_raw(&self) -> String {
        self.lyrics
            .as_ref()
            .map(|d| d.raw_text.clone())
            .unwrap_or_default()
    }

    // ── Visualizer & JSON State Snapshot ─────────────────────────────────────

    pub fn sample_visualizer_floats(&mut self) -> [f32; VISUALIZER_FLOAT_LEN] {
        if self.state == PlaybackState::Playing {
            self.visualizer.read_mono_window(&mut self.fft_scratch);
            let sr = self
                .props
                .as_ref()
                .map(|p| p.source_sample_rate)
                .unwrap_or(48_000)
                .clamp(44_100, 384_000);
            self.spectrum.analyze(&self.fft_scratch, sr);

            let (_active, meters) = self.visualizer.read_meters();
            for (ch, &(p, r)) in meters.iter().enumerate() {
                self.smooth_peaks[ch] = if p > self.smooth_peaks[ch] {
                    self.smooth_peaks[ch] * 0.2 + p * 0.8
                } else {
                    self.smooth_peaks[ch] * 0.82 + p * 0.18
                };
                self.smooth_rms[ch] = self.smooth_rms[ch] * 0.7 + r * 0.3;
                if self.smooth_peaks[ch] > self.peak_hold[ch] {
                    self.peak_hold[ch] = self.smooth_peaks[ch];
                } else {
                    self.peak_hold[ch] = (self.peak_hold[ch] - 0.008).max(self.smooth_peaks[ch]);
                }
            }
        } else {
            self.spectrum.decay_only();
            for ch in 0..MAX_METER_CHANNELS {
                self.smooth_peaks[ch] *= 0.82;
                self.smooth_rms[ch] *= 0.82;
                self.peak_hold[ch] = (self.peak_hold[ch] - 0.015).max(0.0);
            }
        }

        let mut out = [0.0_f32; VISUALIZER_FLOAT_LEN];
        out[0..SPECTRUM_BANDS].copy_from_slice(&self.spectrum.bands);
        out[SPECTRUM_BANDS..SPECTRUM_BANDS * 2].copy_from_slice(&self.spectrum.peaks);
        let m_base = SPECTRUM_BANDS * 2;
        out[m_base..m_base + MAX_METER_CHANNELS].copy_from_slice(&self.smooth_peaks);
        out[m_base + MAX_METER_CHANNELS..m_base + MAX_METER_CHANNELS * 2]
            .copy_from_slice(&self.smooth_rms);
        out[m_base + MAX_METER_CHANNELS * 2..m_base + MAX_METER_CHANNELS * 3]
            .copy_from_slice(&self.peak_hold);
        out
    }

    pub fn snapshot(&mut self) -> MobileSnapshot {
        self.poll_pending_auto_volume();

        let (src_sr, src_ch, upsampled, downmixed, signal_path) = if let Some(ref props) = self.props
        {
            let sp = build_signal_path_str(
                props,
                self.output_rate,
                self.output_channels,
                "Android AudioTrack",
                "32-bit float",
                &self.config,
                &self.auto_volume,
            );
            (
                props.source_sample_rate,
                props.channels,
                self.output_rate > props.decoded_sample_rate,
                self.output_channels < props.channels,
                sp,
            )
        } else {
            (0, 0, false, false, SignalPath::default())
        };

        MobileSnapshot {
            state: self.state,
            position: self.current_position(),
            duration: self.track_info.as_ref().map(|t| t.duration).unwrap_or(0.0),
            source_sample_rate: src_sr,
            output_sample_rate: self.output_rate,
            desired_output_rate: self.desired_output_rate,
            source_channels: src_ch,
            output_channels: self.output_channels,
            upsampled,
            downmixed,
            muted: self.muted,
            track_info: self.track_info.as_ref().map(MobileTrackInfo::from),
            auto_volume: self.auto_volume.clone(),
            signal_path,
            config: self.config.clone(),
            channel_muted: self.mute_solo.muted.to_vec(),
            channel_solo: self.mute_solo.solo.to_vec(),
            active_playlist_id: self.library.active_id.clone(),
            playing_playlist_id: self.playing_playlist_id.clone(),
            current_index: self.current_index,
            playlists: self.library.playlists.clone(),
            lyrics: self.lyrics.clone(),
            lyrics_offset_ms: self.lyrics_offset_ms,
            cover_revision: self.cover_revision,
            error_message: self.error_message.clone(),
            toast: self.take_toast(),
        }
    }
}

impl Default for SonoraMobileEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_engine_renders_audio_cover_and_playlists() {
        let mut eng = SonoraMobileEngine::new_in_memory();

        // Pre-staged Studio Reference track 01 + cover art PNG
        assert!(eng.track_info.is_some());
        assert!(eng.cover_art_bytes().map(|b| b.len() > 100).unwrap_or(false));

        // Play and pull interleaved 32-bit float audio
        eng.play();
        let mut buf = vec![0.0_f32; 4096];
        let got = eng.render_audio(&mut buf);
        assert_eq!(got as usize, buf.len());
        let peak = buf.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        assert!(peak > 0.001, "expected audible output, peak={}", peak);

        // Create playlist and serialize state JSON
        let pl_id = eng.library.create_playlist("Phone Hi-Res", Vec::new());
        assert_eq!(eng.library.active_id, pl_id);
        let snap = eng.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("Phone Hi-Res"));
        assert!(json.contains("Nocturne in Amber"));
    }
}
