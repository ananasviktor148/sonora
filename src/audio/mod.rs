pub mod android_engine;
pub mod decoder;
pub mod demo;
pub mod dsd;
pub mod dsp;
pub mod loudness;
pub mod ringbuf;

use serde::Serialize;

use crate::audio::decoder::{channel_layout_label, StreamProperties};
use crate::config::{format_rate, AppConfig, AutoVolumeMode, DownmixMode, UpsampleMode};

// ── Cross-Platform Public Types & Signal Path Helpers ────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AutoVolumeStatus {
    pub active: bool,
    pub analyzing: bool,
    pub mode: AutoVolumeMode,
    pub applied_gain_db: f32,
    pub measured_lufs: Option<f32>,
    pub peak_dbfs: Option<f32>,
    pub peak_limited: bool,
    pub bypassed_for_dop: bool,
    pub source_label: String,
}

impl Default for AutoVolumeStatus {
    fn default() -> Self {
        Self {
            active: false,
            analyzing: false,
            mode: AutoVolumeMode::Off,
            applied_gain_db: 0.0,
            measured_lufs: None,
            peak_dbfs: None,
            peak_limited: false,
            bypassed_for_dop: false,
            source_label: "Off".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalPathStage {
    pub title: String,
    pub detail: String,
    pub lossless: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SignalPath {
    pub stages: Vec<SignalPathStage>,
    pub bit_perfect: bool,
}

#[inline]
pub fn db_to_lin_gain(db: f32) -> f32 {
    10.0_f32.powf(db.clamp(-24.0, 18.0) / 20.0)
}

#[inline]
pub fn output_soft_clip(x: f32, soft_clip: bool) -> f32 {
    if soft_clip {
        let a = x.abs();
        if a <= 0.96 {
            x
        } else {
            let excess = (a - 0.96) / 0.04;
            x.signum() * (0.96 + 0.04 * excess.tanh())
        }
    } else {
        x.clamp(-1.0, 1.0)
    }
}

pub fn build_signal_path_str(
    props: &StreamProperties,
    output_rate: u32,
    output_channels: u16,
    device_name: &str,
    fmt_str: &str,
    config: &AppConfig,
    auto_vol: &AutoVolumeStatus,
) -> SignalPath {
    let mut stages = Vec::new();

    // Stage 1: Source
    let src_detail = if props.is_dsd {
        format!(
            "{} {} ({} · 1-bit · {}) via {}",
            props.format_name,
            props.codec_name,
            format_rate(props.source_sample_rate),
            channel_layout_label(props.channels),
            props.backend
        )
    } else {
        format!(
            "{} {} ({} · {}-bit · {}) via {}",
            props.format_name,
            props.codec_name,
            format_rate(props.source_sample_rate),
            props.bit_depth,
            channel_layout_label(props.channels),
            props.backend
        )
    };
    stages.push(SignalPathStage {
        title: "Source".into(),
        detail: src_detail,
        lossless: true,
    });

    // Stage 2: DSD Engine
    if props.is_dsd {
        if props.is_dop {
            stages.push(SignalPathStage {
                title: "DSD DoP v1.1".into(),
                detail: format!(
                    "Bit-exact DoP 0x05/0xFA packing @ {} 24-bit",
                    format_rate(output_rate)
                ),
                lossless: true,
            });
        } else {
            stages.push(SignalPathStage {
                title: "DSD FIR Decimator".into(),
                detail: format!(
                    "Blackman-Harris Polyphase -> {} 32f (LPF {} kHz{})",
                    format_rate(props.decoded_sample_rate),
                    config.dsd.cutoff_hz / 1000,
                    if config.dsd.gain_boost_6db {
                        ", +6 dB"
                    } else {
                        ""
                    }
                ),
                lossless: true,
            });
        }
    }

    // Stage 3: Upsampler / Resampler
    if !props.is_dop && output_rate != props.decoded_sample_rate {
        let dir = if output_rate > props.decoded_sample_rate {
            "Upsampler"
        } else {
            "DAC Rate Match"
        };
        stages.push(SignalPathStage {
            title: dir.into(),
            detail: format!(
                "{} -> {} · 64-bit Sinc ({})",
                format_rate(props.decoded_sample_rate),
                format_rate(output_rate),
                config.resampler_quality.label()
            ),
            lossless: true,
        });
    }

    // Stage 4: Equalizer
    if !props.is_dop && config.eq.enabled {
        stages.push(SignalPathStage {
            title: "10-Band EQ".into(),
            detail: format!(
                "64-bit Biquad · Preset: {} (Preamp {:+.1} dB)",
                config.eq.preset_name, config.eq.preamp_db
            ),
            lossless: false,
        });
    }

    // Stage 5: Multi-Channel Router / Downmix
    if !props.is_dop
        && (output_channels != props.channels
            || config.multichannel.downmix_mode == DownmixMode::HeadphoneSpatial)
    {
        stages.push(SignalPathStage {
            title: "Channel Matrix".into(),
            detail: format!(
                "{} -> {} ({})",
                channel_layout_label(props.channels),
                channel_layout_label(output_channels),
                config.multichannel.downmix_mode.label()
            ),
            lossless: output_channels >= props.channels,
        });
    }

    // Stage 5.5: Auto Adjust Playback Volume (AudioGate Loudness Normalization)
    if config.auto_volume.mode != AutoVolumeMode::Off {
        if props.is_dop {
            stages.push(SignalPathStage {
                title: "Auto Volume".into(),
                detail: format!(
                    "{} Mode · Bypassed for Bit-Exact DoP Output",
                    config.auto_volume.mode.short_label()
                ),
                lossless: true,
            });
        } else if auto_vol.analyzing {
            stages.push(SignalPathStage {
                title: "Auto Volume".into(),
                detail: format!(
                    "{} Mode · Measuring ITU-R BS.1770-4 Loudness (Target {:.1} LUFS)…",
                    config.auto_volume.mode.short_label(),
                    config.auto_volume.target_lufs
                ),
                lossless: true,
            });
        } else if auto_vol.active {
            let meas_txt = auto_vol
                .measured_lufs
                .map(|l| format!("Measured {:.1} LUFS · ", l))
                .unwrap_or_default();
            let lim_txt = if auto_vol.peak_limited {
                " [Peak-Safe]"
            } else {
                ""
            };
            stages.push(SignalPathStage {
                title: "Auto Volume".into(),
                detail: format!(
                    "{} Mode · {:+.1} dB{} ({}Target {:.1} LUFS · {})",
                    config.auto_volume.mode.short_label(),
                    auto_vol.applied_gain_db,
                    lim_txt,
                    meas_txt,
                    config.auto_volume.target_lufs,
                    auto_vol.source_label
                ),
                lossless: auto_vol.applied_gain_db.abs() < 0.05,
            });
        }
    }

    // Stage 6: Output DAC
    stages.push(SignalPathStage {
        title: "Audio Output".into(),
        detail: format!(
            "{} · {} · {} · {}",
            device_name,
            format_rate(output_rate),
            fmt_str,
            channel_layout_label(output_channels)
        ),
        lossless: true,
    });

    let auto_vol_unity = config.auto_volume.mode == AutoVolumeMode::Off
        || !auto_vol.active
        || auto_vol.applied_gain_db.abs() < 0.05;

    let bit_perfect = props.is_dop
        || (!props.is_dsd
            && config.upsample_mode == UpsampleMode::Bypass
            && output_rate == props.source_sample_rate
            && output_channels == props.channels
            && !config.eq.enabled
            && config.headroom_db.abs() < 0.01
            && auto_vol_unity);

    SignalPath {
        stages,
        bit_perfect,
    }
}

#[cfg(not(target_os = "android"))]
mod desktop {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use anyhow::{anyhow, Result};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, StreamConfig};
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use parking_lot::RwLock;

    use crate::audio::decoder::{
        is_ffmpeg_available, open_decoder, probe_track_info, StreamProperties, StreamingDecoder,
        TrackInfo,
    };
    use crate::audio::dsp::{ChannelMuteSolo, MultiChannelRouter, StreamingUpsampler, TenBandEq};
    use crate::audio::ringbuf::{AudioRingBuffer, VisualizerTap};
    use crate::config::{AppConfig, AutoVolumeMode, DownmixMode};

    #[derive(Debug, Clone)]
    pub enum AudioCommand {
        Load {
            path: PathBuf,
            start_at: f64,
            autoplay: bool,
        },
        Play,
        Pause,
        Stop,
        Seek(f64),
        SetVolume(f32),
        SetMuted(bool),
        UpdateConfig(AppConfig),
        SetChannelMuteSolo(ChannelMuteSolo),
        SetAutoVolumeAnalyzing(AutoVolumeMode),
        SetAutoVolume {
            gain_db: f32,
            status: AutoVolumeStatus,
        },
        RefreshDevices,
    }

    #[derive(Debug, Clone)]
    pub struct AudioDeviceInfo {
        pub name: String,
        pub is_default: bool,
        pub max_channels: u16,
        pub min_sample_rate: u32,
        pub max_sample_rate: u32,
    }

    #[derive(Debug, Clone)]
    pub struct PlaybackStatus {
        pub state: PlaybackState,
        pub position: f64,
        pub duration: f64,
        pub source_sample_rate: u32,
        pub output_sample_rate: u32,
        pub source_channels: u16,
        pub output_channels: u16,
        pub output_format_label: String,
        pub output_device_name: String,
        pub is_dsd: bool,
        pub is_dop: bool,
        pub upsampled: bool,
        pub downmixed: bool,
        pub auto_volume: AutoVolumeStatus,
        pub error_message: Option<String>,
        pub signal_path: SignalPath,
    }

    impl Default for PlaybackStatus {
        fn default() -> Self {
            Self {
                state: PlaybackState::Stopped,
                position: 0.0,
                duration: 0.0,
                source_sample_rate: 0,
                output_sample_rate: 0,
                source_channels: 0,
                output_channels: 0,
                output_format_label: String::new(),
                output_device_name: String::new(),
                is_dsd: false,
                is_dop: false,
                upsampled: false,
                downmixed: false,
                auto_volume: AutoVolumeStatus::default(),
                error_message: None,
                signal_path: SignalPath::default(),
            }
        }
    }

    // ── Shared Lock-Free Callback Control ────────────────────────────────────

    struct CallbackControl {
        playing: AtomicBool,
        eof_reached: AtomicBool,
        track_finished: AtomicBool,
        volume_bits: AtomicU32,
        auto_gain_bits: AtomicU32,
        soft_clip: AtomicBool,
        muted: AtomicBool,
        dop_mode: AtomicBool,
        played_frames: AtomicU64,
    }

    impl CallbackControl {
        fn new(volume: f32, soft_clip: bool) -> Self {
            Self {
                playing: AtomicBool::new(false),
                eof_reached: AtomicBool::new(false),
                track_finished: AtomicBool::new(false),
                volume_bits: AtomicU32::new(volume.to_bits()),
                auto_gain_bits: AtomicU32::new(1.0_f32.to_bits()),
                soft_clip: AtomicBool::new(soft_clip),
                muted: AtomicBool::new(false),
                dop_mode: AtomicBool::new(false),
                played_frames: AtomicU64::new(0),
            }
        }

        fn effective_user_gain(&self) -> f32 {
            if self.dop_mode.load(Ordering::Relaxed) {
                return 1.0; // Must be bit-exact unity for DoP markers
            }
            if self.muted.load(Ordering::Relaxed) {
                return 0.0;
            }
            f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
        }

        fn target_auto_gain(&self) -> f32 {
            if self.dop_mode.load(Ordering::Relaxed) {
                return 1.0;
            }
            let g = f32::from_bits(self.auto_gain_bits.load(Ordering::Relaxed));
            if g.is_finite() && g > 0.0 {
                g.clamp(0.05, 10.0)
            } else {
                1.0
            }
        }
    }

struct SharedState {
    status: PlaybackStatus,
    track_info: Option<TrackInfo>,
    devices: Vec<AudioDeviceInfo>,
    ffmpeg_available: bool,
}

// ── AudioEngine Public Handle ────────────────────────────────────────────────

pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    shared: Arc<RwLock<SharedState>>,
    callback_ctrl: Arc<CallbackControl>,
    visualizer: Arc<VisualizerTap>,
}

impl AudioEngine {
    pub fn new(initial_config: AppConfig) -> Self {
        let (cmd_tx, cmd_rx) = unbounded();
        let devices = enumerate_output_devices();
        let ffmpeg_available = is_ffmpeg_available();

        let shared = Arc::new(RwLock::new(SharedState {
            status: PlaybackStatus::default(),
            track_info: None,
            devices,
            ffmpeg_available,
        }));
        let callback_ctrl = Arc::new(CallbackControl::new(
            initial_config.volume,
            initial_config.soft_clip,
        ));
        let visualizer = Arc::new(VisualizerTap::new());

        let shared_c = Arc::clone(&shared);
        let ctrl_c = Arc::clone(&callback_ctrl);
        let vis_c = Arc::clone(&visualizer);

        thread::Builder::new()
            .name("hires-audio-engine".into())
            .spawn(move || {
                audio_worker_loop(cmd_rx, shared_c, ctrl_c, vis_c, initial_config);
            })
            .expect("Failed to spawn audio engine thread");

        Self {
            cmd_tx,
            shared,
            callback_ctrl,
            visualizer,
        }
    }

    pub fn send(&self, cmd: AudioCommand) {
        // Apply volume, mute, & soft-clip atomically right away for zero UI latency
        match &cmd {
            AudioCommand::SetVolume(v) => {
                self.callback_ctrl
                    .volume_bits
                    .store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
            }
            AudioCommand::SetMuted(m) => {
                self.callback_ctrl.muted.store(*m, Ordering::Relaxed);
            }
            AudioCommand::UpdateConfig(cfg) => {
                self.callback_ctrl
                    .soft_clip
                    .store(cfg.soft_clip, Ordering::Relaxed);
            }
            AudioCommand::SetAutoVolumeAnalyzing(_) => {
                self.callback_ctrl
                    .auto_gain_bits
                    .store(1.0_f32.to_bits(), Ordering::Relaxed);
            }
            AudioCommand::SetAutoVolume { gain_db, status } => {
                let lin = if status.active
                    && !status.bypassed_for_dop
                    && status.mode != AutoVolumeMode::Off
                {
                    db_to_lin_gain(*gain_db)
                } else {
                    1.0
                };
                self.callback_ctrl
                    .auto_gain_bits
                    .store(lin.to_bits(), Ordering::Relaxed);
            }
            _ => {}
        }
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn command_sender(&self) -> Sender<AudioCommand> {
        self.cmd_tx.clone()
    }

    pub fn status(&self) -> PlaybackStatus {
        self.shared.read().status.clone()
    }

    pub fn track_info(&self) -> Option<TrackInfo> {
        self.shared.read().track_info.clone()
    }

    pub fn devices(&self) -> Vec<AudioDeviceInfo> {
        self.shared.read().devices.clone()
    }

    pub fn ffmpeg_available(&self) -> bool {
        self.shared.read().ffmpeg_available
    }

    pub fn visualizer(&self) -> &Arc<VisualizerTap> {
        &self.visualizer
    }
}

// ── Device Enumeration & Config Selection ────────────────────────────────────

pub fn enumerate_output_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let mut list = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for dev in devices {
            let name = match dev.name() {
                Ok(n) => n,
                Err(_) => continue,
            };
            let mut max_ch = 2u16;
            let mut min_sr = 44_100u32;
            let mut max_sr = 48_000u32;
            let mut found_cfg = false;

            if let Ok(configs) = dev.supported_output_configs() {
                for c in configs {
                    if !found_cfg {
                        max_ch = c.channels();
                        min_sr = c.min_sample_rate().0;
                        max_sr = c.max_sample_rate().0;
                        found_cfg = true;
                    } else {
                        max_ch = max_ch.max(c.channels());
                        min_sr = min_sr.min(c.min_sample_rate().0);
                        max_sr = max_sr.max(c.max_sample_rate().0);
                    }
                }
            }
            if !found_cfg {
                if let Ok(def) = dev.default_output_config() {
                    max_ch = def.channels();
                    min_sr = def.sample_rate().0;
                    max_sr = def.sample_rate().0;
                }
            }

            list.push(AudioDeviceInfo {
                is_default: name == default_name,
                name,
                max_channels: max_ch,
                min_sample_rate: min_sr,
                max_sample_rate: max_sr,
            });
        }
    }

    // Put default device first
    list.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
    list
}

fn select_cpal_device(preferred_name: &Option<String>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(target) = preferred_name {
        if let Ok(mut devs) = host.output_devices() {
            if let Some(d) = devs.find(|d| d.name().ok().as_deref() == Some(target.as_str())) {
                return Ok(d);
            }
        }
    }
    host.default_output_device()
        .ok_or_else(|| anyhow!("No audio output device available on this system"))
}

struct ChosenOutput {
    config: StreamConfig,
    sample_format: SampleFormat,
}

fn choose_output_config(
    device: &cpal::Device,
    desired_rate: u32,
    source_channels: u16,
    downmix_mode: DownmixMode,
    is_dop: bool,
) -> Result<ChosenOutput> {
    let desired_channels: u16 = if is_dop {
        source_channels
    } else {
        match downmix_mode {
            DownmixMode::StereoItu | DownmixMode::HeadphoneSpatial => 2,
            DownmixMode::Auto | DownmixMode::NativePassthrough => source_channels,
        }
    };

    let supported: Vec<_> = device
        .supported_output_configs()
        .map(|it| it.collect())
        .unwrap_or_default();

    let format_priority = |fmt: SampleFormat| -> i32 {
        match fmt {
            SampleFormat::F32 => 4,
            SampleFormat::I32 => 3,
            SampleFormat::I16 => 2,
            SampleFormat::U16 => 1,
            _ => 0,
        }
    };

    // Score each supported config range
    let mut best: Option<(i64, ChosenOutput)> = None;
    for range in &supported {
        let fmt = range.sample_format();
        let fmt_score = format_priority(fmt);
        if fmt_score == 0 {
            continue;
        }

        let ch = range.channels();
        let ch_score: i64 = if ch == desired_channels {
            1000
        } else if desired_channels > 2 && ch == 6 && desired_channels == 8 {
            800
        } else if ch == 2 {
            600
        } else if ch >= desired_channels {
            500 - (ch - desired_channels) as i64 * 10
        } else {
            200 + ch as i64 * 10
        };

        let min_r = range.min_sample_rate().0;
        let max_r = range.max_sample_rate().0;
        let chosen_rate = desired_rate.clamp(min_r, max_r);
        let rate_diff = (chosen_rate as i64 - desired_rate as i64).abs();
        let rate_score: i64 = if rate_diff == 0 {
            1000
        } else {
            // Prefer higher rates over downsampling when exact isn't available
            let penalty = (rate_diff / 100).min(800);
            800 - penalty + if chosen_rate >= desired_rate { 50 } else { 0 }
        };

        let total_score = ch_score * 10 + rate_score * 5 + fmt_score as i64;
        let candidate = ChosenOutput {
            config: StreamConfig {
                channels: ch,
                sample_rate: cpal::SampleRate(chosen_rate),
                buffer_size: cpal::BufferSize::Default,
            },
            sample_format: fmt,
        };

        if best.as_ref().map(|(s, _)| total_score > *s).unwrap_or(true) {
            best = Some((total_score, candidate));
        }
    }

    if let Some((_, chosen)) = best {
        return Ok(chosen);
    }

    // Fallback to device default output config
    let def = device.default_output_config()?;
    let fmt = def.sample_format();
    Ok(ChosenOutput {
        config: def.into(),
        sample_format: fmt,
    })
}

#[allow(clippy::too_many_arguments)]
fn fill_cpal_buffer<T: Copy>(
    data: &mut [T],
    silence: T,
    convert: impl Fn(f32) -> T,
    scratch: &mut Vec<f32>,
    ring: &AudioRingBuffer,
    ctrl: &CallbackControl,
    channels: usize,
    ramp_coeff: f32,
    smooth_auto_gain: &mut f32,
) {
    if !ctrl.playing.load(Ordering::Relaxed) {
        data.fill(silence);
        *smooth_auto_gain = ctrl.target_auto_gain();
        return;
    }
    if scratch.len() < data.len() {
        scratch.resize(data.len(), 0.0);
    }
    let got = ring.pop(&mut scratch[..data.len()]);
    let is_dop = ctrl.dop_mode.load(Ordering::Relaxed);
    if is_dop {
        for (dst, &src) in data.iter_mut().zip(scratch.iter()).take(got) {
            *dst = convert(src.clamp(-1.0, 1.0));
        }
    } else {
        let user_gain = ctrl.effective_user_gain();
        let target_auto = ctrl.target_auto_gain();
        let soft_clip = ctrl.soft_clip.load(Ordering::Relaxed);
        let frames = got / channels;
        for f in 0..frames {
            *smooth_auto_gain += (target_auto - *smooth_auto_gain) * ramp_coeff;
            let total_gain = user_gain * *smooth_auto_gain;
            let base = f * channels;
            for c in 0..channels {
                data[base + c] = convert(output_soft_clip(scratch[base + c] * total_gain, soft_clip));
            }
        }
        for i in (frames * channels)..got {
            data[i] = convert(output_soft_clip(
                scratch[i] * user_gain * *smooth_auto_gain,
                soft_clip,
            ));
        }
    }
    if got < data.len() {
        data[got..].fill(silence);
        if got == 0 && ctrl.eof_reached.load(Ordering::Acquire) {
            ctrl.track_finished.store(true, Ordering::Release);
            ctrl.playing.store(false, Ordering::Relaxed);
        }
    }
    if got > 0 {
        ctrl.played_frames
            .fetch_add((got / channels) as u64, Ordering::Relaxed);
    }
}

fn build_cpal_stream(
    device: &cpal::Device,
    chosen: &ChosenOutput,
    ring: Arc<AudioRingBuffer>,
    ctrl: Arc<CallbackControl>,
) -> Result<cpal::Stream> {
    let channels = (chosen.config.channels as usize).max(1);
    let sample_rate = chosen.config.sample_rate.0.max(8_000) as f32;
    // Smooth ~45 ms exponential gain ramp to eliminate clicks when switching modes or tracks
    let ramp_coeff = (1.0 - (-1.0 / (sample_rate * 0.045)).exp()).clamp(0.0001, 1.0);
    let err_fn = |err| log::error!("Audio output stream error: {}", err);

    let stream = match chosen.sample_format {
        SampleFormat::F32 => {
            let mut scratch = vec![0.0f32; 8192];
            let mut smooth_auto_gain = ctrl.target_auto_gain();
            device.build_output_stream(
                &chosen.config,
                move |data: &mut [f32], _| {
                    fill_cpal_buffer(
                        data,
                        0.0f32,
                        |s| s,
                        &mut scratch,
                        &ring,
                        &ctrl,
                        channels,
                        ramp_coeff,
                        &mut smooth_auto_gain,
                    );
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::I32 => {
            let mut scratch = vec![0.0f32; 8192];
            let mut smooth_auto_gain = ctrl.target_auto_gain();
            device.build_output_stream(
                &chosen.config,
                move |data: &mut [i32], _| {
                    fill_cpal_buffer(
                        data,
                        0i32,
                        |s| (s as f64 * i32::MAX as f64) as i32,
                        &mut scratch,
                        &ring,
                        &ctrl,
                        channels,
                        ramp_coeff,
                        &mut smooth_auto_gain,
                    );
                },
                err_fn,
                None,
            )?
        }
        SampleFormat::I16 => {
            let mut scratch = vec![0.0f32; 8192];
            let mut smooth_auto_gain = ctrl.target_auto_gain();
            device.build_output_stream(
                &chosen.config,
                move |data: &mut [i16], _| {
                    fill_cpal_buffer(
                        data,
                        0i16,
                        |s| (s * i16::MAX as f32) as i16,
                        &mut scratch,
                        &ring,
                        &ctrl,
                        channels,
                        ramp_coeff,
                        &mut smooth_auto_gain,
                    );
                },
                err_fn,
                None,
            )?
        }
        other => {
            return Err(anyhow!("Unsupported CPAL output sample format: {:?}", other));
        }
    };

    stream.play()?;
    Ok(stream)
}

// ── Engine Worker Loop ───────────────────────────────────────────────────────

struct ActivePipeline {
    path: PathBuf,
    decoder: Option<Box<dyn StreamingDecoder>>,
    props: StreamProperties,
    upsampler: Option<StreamingUpsampler>,
    eq: TenBandEq,
    router: MultiChannelRouter,
    ring: Arc<AudioRingBuffer>,
    _stream: cpal::Stream,
    output_rate: u32,
    output_channels: u16,
    device_name: String,
    sample_format: SampleFormat,
    seek_base_secs: f64,
    pending_interleaved: Vec<f32>,
    pending_offset: usize,
}

fn audio_worker_loop(
    cmd_rx: Receiver<AudioCommand>,
    shared: Arc<RwLock<SharedState>>,
    ctrl: Arc<CallbackControl>,
    visualizer: Arc<VisualizerTap>,
    mut config: AppConfig,
) {
    let mut pipeline: Option<ActivePipeline> = None;
    let mut mute_solo = ChannelMuteSolo::default();

    loop {
        // Check if CPAL callback finished draining EOF
        if ctrl.track_finished.swap(false, Ordering::AcqRel) {
            visualizer.reset_meters();
            let mut s = shared.write();
            s.status.state = PlaybackState::Finished;
            s.status.position = s.status.duration;
        }

        // Update live playback position from DAC frame counter
        if let Some(ref pipe) = pipeline {
            if ctrl.playing.load(Ordering::Relaxed) && pipe.output_rate > 0 {
                let played = ctrl.played_frames.load(Ordering::Relaxed);
                let pos = pipe.seek_base_secs + (played as f64 / pipe.output_rate as f64);
                let mut s = shared.write();
                s.status.position = if s.status.duration > 0.0 {
                    pos.min(s.status.duration)
                } else {
                    pos
                };
            }
        }

        // Process pending commands (block briefly if idle or ring buffer already well-fed)
        let should_decode = pipeline
            .as_ref()
            .map(|p| {
                ctrl.playing.load(Ordering::Relaxed)
                    && (p.decoder.is_some() || p.pending_offset < p.pending_interleaved.len())
                    && p.ring.available_write() >= (p.output_channels as usize * 1024)
            })
            .unwrap_or(false);

        let cmd_opt = if should_decode {
            cmd_rx.try_recv().ok()
        } else {
            match cmd_rx.recv_timeout(Duration::from_millis(12)) {
                Ok(c) => Some(c),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        };

        if let Some(cmd) = cmd_opt {
            handle_command(
                cmd,
                &mut pipeline,
                &mut config,
                &mut mute_solo,
                &shared,
                &ctrl,
                &visualizer,
            );
            // Drain any rapid consecutive commands before pumping DSP
            while let Ok(more) = cmd_rx.try_recv() {
                handle_command(
                    more,
                    &mut pipeline,
                    &mut config,
                    &mut mute_solo,
                    &shared,
                    &ctrl,
                    &visualizer,
                );
            }
        }

        // Pump one streaming decode + DSP + ringbuf cycle
        if let Some(ref mut pipe) = pipeline {
            if !ctrl.playing.load(Ordering::Relaxed) {
                continue;
            }

            // First push any leftover interleaved samples from previous chunk
            if pipe.pending_offset < pipe.pending_interleaved.len() {
                let written = pipe
                    .ring
                    .push(&pipe.pending_interleaved[pipe.pending_offset..]);
                pipe.pending_offset += written;
                if pipe.pending_offset < pipe.pending_interleaved.len() {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                pipe.pending_interleaved.clear();
                pipe.pending_offset = 0;
            }

            if pipe.ring.available_write() < (pipe.output_channels as usize * 1024) {
                continue;
            }

            // Decode next streaming chunk
            let next = if let Some(ref mut dec) = pipe.decoder {
                dec.next_chunk()
            } else {
                continue;
            };

            match next {
                Ok(Some(mut planar)) => {
                    // 1. Apply per-channel mute/solo
                    MultiChannelRouter::apply_mute_solo(&mut planar, &mute_solo);

                    // 2. Tap real-time visualizer & per-channel VU meters
                    visualizer.push_planar(&planar);

                    // 3. Optional audiophile upsampling / rate adaptation
                    let mut processed = if let Some(ref mut up) = pipe.upsampler {
                        match up.process(&planar) {
                            Ok(resampled) => resampled,
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

                    // 4. Optional 10-band 64-bit EQ (bypassed in DoP mode)
                    if !pipe.props.is_dop {
                        pipe.eq
                            .update(pipe.output_rate, processed.len(), &config.eq);
                        pipe.eq.process_in_place(&mut processed, &config.eq);
                    }

                    // 5. Multi-channel routing / ITU-R BS.775 downmix / Crossfeed / Headroom
                    let interleaved = pipe.router.mix_to_interleaved(
                        &processed,
                        pipe.output_channels as usize,
                        &config.multichannel,
                        config.headroom_db,
                        config.soft_clip,
                        pipe.props.is_dop,
                    );

                    // 6. Push into lock-free ring buffer
                    let written = pipe.ring.push(&interleaved);
                    if written < interleaved.len() {
                        pipe.pending_interleaved = interleaved;
                        pipe.pending_offset = written;
                    }
                }
                Ok(None) => {
                    // Flush resampler tail on clean EOF
                    if let Some(ref mut up) = pipe.upsampler {
                        if let Ok(mut tail) = up.flush() {
                            if tail.first().map(|c| !c.is_empty()).unwrap_or(false) {
                                if !pipe.props.is_dop {
                                    pipe.eq.process_in_place(&mut tail, &config.eq);
                                }
                                let interleaved = pipe.router.mix_to_interleaved(
                                    &tail,
                                    pipe.output_channels as usize,
                                    &config.multichannel,
                                    config.headroom_db,
                                    config.soft_clip,
                                    pipe.props.is_dop,
                                );
                                let written = pipe.ring.push(&interleaved);
                                if written < interleaved.len() {
                                    pipe.pending_interleaved = interleaved;
                                    pipe.pending_offset = written;
                                }
                            }
                        }
                    }
                    pipe.decoder = None;
                    ctrl.eof_reached.store(true, Ordering::Release);
                }
                Err(e) => {
                    log::error!("Decode error during playback: {}", e);
                    pipe.decoder = None;
                    ctrl.eof_reached.store(true, Ordering::Release);
                }
            }
        }
    }
}

fn handle_command(
    cmd: AudioCommand,
    pipeline: &mut Option<ActivePipeline>,
    config: &mut AppConfig,
    mute_solo: &mut ChannelMuteSolo,
    shared: &Arc<RwLock<SharedState>>,
    ctrl: &Arc<CallbackControl>,
    visualizer: &Arc<VisualizerTap>,
) {
    match cmd {
        AudioCommand::Load {
            path,
            start_at,
            autoplay,
        } => {
            // Stop previous stream cleanly
            ctrl.playing.store(false, Ordering::SeqCst);
            *pipeline = None;
            visualizer.reset_meters();

            match open_and_start_pipeline(&path, start_at, autoplay, config, ctrl) {
                Ok((pipe, info, status)) => {
                    let mut s = shared.write();
                    s.track_info = Some(info);
                    s.status = status;
                    *pipeline = Some(pipe);
                }
                Err(e) => {
                    log::error!("Failed to load {}: {}", path.display(), e);
                    let mut s = shared.write();
                    s.status.state = PlaybackState::Stopped;
                    s.status.error_message = Some(format!("Playback error: {}", e));
                }
            }
        }

        AudioCommand::Play => {
            if pipeline.is_some() {
                ctrl.playing.store(true, Ordering::SeqCst);
                shared.write().status.state = PlaybackState::Playing;
            }
        }

        AudioCommand::Pause => {
            ctrl.playing.store(false, Ordering::SeqCst);
            visualizer.reset_meters();
            if pipeline.is_some() {
                shared.write().status.state = PlaybackState::Paused;
            }
        }

        AudioCommand::Stop => {
            ctrl.playing.store(false, Ordering::SeqCst);
            ctrl.eof_reached.store(false, Ordering::SeqCst);
            ctrl.track_finished.store(false, Ordering::SeqCst);
            ctrl.played_frames.store(0, Ordering::SeqCst);
            *pipeline = None;
            visualizer.reset_meters();
            let mut s = shared.write();
            s.status.state = PlaybackState::Stopped;
            s.status.position = 0.0;
        }

        AudioCommand::Seek(target_secs) => {
            if let Some(ref mut pipe) = pipeline {
                let was_playing = ctrl.playing.load(Ordering::Relaxed);
                let clamped = target_secs.clamp(0.0, pipe.props.duration.max(0.0));

                // Pause callback briefly while flushing ring buffer
                ctrl.playing.store(false, Ordering::SeqCst);
                pipe.ring.clear();
                pipe.pending_interleaved.clear();
                pipe.pending_offset = 0;

                // Re-open decoder if it had already reached EOF
                if pipe.decoder.is_none() {
                    if let Ok(dec) = open_decoder(&pipe.path, &config.dsd) {
                        pipe.decoder = Some(dec);
                    }
                }

                if let Some(ref mut dec) = pipe.decoder {
                    if let Err(e) = dec.seek(clamped) {
                        log::warn!("Seek failed, reopening at {}: {}", clamped, e);
                        if let Ok(mut fresh) = open_decoder(&pipe.path, &config.dsd) {
                            let _ = fresh.seek(clamped);
                            *dec = fresh;
                        }
                    }
                }

                if let Some(ref mut up) = pipe.upsampler {
                    up.reset();
                }
                pipe.eq.reset();
                pipe.router.reset();
                pipe.seek_base_secs = clamped;
                ctrl.played_frames.store(0, Ordering::SeqCst);
                ctrl.eof_reached.store(false, Ordering::SeqCst);
                ctrl.track_finished.store(false, Ordering::SeqCst);
                ctrl.playing.store(was_playing, Ordering::SeqCst);

                shared.write().status.position = clamped;
            }
        }

        AudioCommand::SetVolume(v) => {
            config.volume = v.clamp(0.0, 1.0);
        }

        AudioCommand::SetMuted(_) => {}

        AudioCommand::SetChannelMuteSolo(ms) => {
            *mute_solo = ms;
        }

        AudioCommand::SetAutoVolumeAnalyzing(mode) => {
            ctrl.auto_gain_bits
                .store(1.0_f32.to_bits(), Ordering::Relaxed);
            let is_dop = pipeline.as_ref().map(|p| p.props.is_dop).unwrap_or(false);
            let av_status = AutoVolumeStatus {
                active: false,
                analyzing: !is_dop && mode != AutoVolumeMode::Off,
                mode,
                applied_gain_db: 0.0,
                measured_lufs: None,
                peak_dbfs: None,
                peak_limited: false,
                bypassed_for_dop: is_dop && mode != AutoVolumeMode::Off,
                source_label: if is_dop {
                    "Bypassed (DoP)".to_string()
                } else {
                    "Analyzing…".to_string()
                },
            };
            let mut s = shared.write();
            s.status.auto_volume = av_status.clone();
            if let Some(ref pipe) = pipeline {
                s.status.signal_path = build_signal_path(
                    &pipe.props,
                    pipe.output_rate,
                    pipe.output_channels,
                    &pipe.device_name,
                    pipe.sample_format,
                    config,
                    &av_status,
                );
            }
        }

        AudioCommand::SetAutoVolume { gain_db, mut status } => {
            let is_dop = pipeline.as_ref().map(|p| p.props.is_dop).unwrap_or(false);
            if is_dop && status.mode != AutoVolumeMode::Off {
                status.active = false;
                status.analyzing = false;
                status.applied_gain_db = 0.0;
                status.bypassed_for_dop = true;
                status.source_label = "Bypassed (DoP)".to_string();
                ctrl.auto_gain_bits
                    .store(1.0_f32.to_bits(), Ordering::Relaxed);
            } else if status.mode == AutoVolumeMode::Off || !status.active {
                status.applied_gain_db = 0.0;
                ctrl.auto_gain_bits
                    .store(1.0_f32.to_bits(), Ordering::Relaxed);
            } else {
                let lin = db_to_lin_gain(gain_db);
                ctrl.auto_gain_bits.store(lin.to_bits(), Ordering::Relaxed);
            }

            let mut s = shared.write();
            s.status.auto_volume = status.clone();
            if let Some(ref pipe) = pipeline {
                s.status.signal_path = build_signal_path(
                    &pipe.props,
                    pipe.output_rate,
                    pipe.output_channels,
                    &pipe.device_name,
                    pipe.sample_format,
                    config,
                    &status,
                );
            }
        }

        AudioCommand::UpdateConfig(new_cfg) => {
            let needs_reopen = config.selected_device != new_cfg.selected_device
                || config.upsample_mode != new_cfg.upsample_mode
                || config.resampler_quality != new_cfg.resampler_quality
                || config.filter_rolloff != new_cfg.filter_rolloff
                || config.dsd != new_cfg.dsd
                || config.multichannel.downmix_mode != new_cfg.multichannel.downmix_mode;

            *config = new_cfg;
            ctrl.soft_clip.store(config.soft_clip, Ordering::Relaxed);
            if config.auto_volume.mode == AutoVolumeMode::Off {
                ctrl.auto_gain_bits
                    .store(1.0_f32.to_bits(), Ordering::Relaxed);
            }

            if let Some(ref pipe) = pipeline {
                if needs_reopen {
                    let path = pipe.path.clone();
                    let was_playing = ctrl.playing.load(Ordering::Relaxed);
                    let current_pos = shared.read().status.position;
                    let prev_av = shared.read().status.auto_volume.clone();

                    ctrl.playing.store(false, Ordering::SeqCst);
                    *pipeline = None;

                    if let Ok((new_pipe, info, mut status)) =
                        open_and_start_pipeline(&path, current_pos, was_playing, config, ctrl)
                    {
                        if config.auto_volume.mode != AutoVolumeMode::Off {
                            let mut av = prev_av;
                            if new_pipe.props.is_dop {
                                av.active = false;
                                av.bypassed_for_dop = true;
                                av.applied_gain_db = 0.0;
                                av.source_label = "Bypassed (DoP)".to_string();
                                ctrl.auto_gain_bits
                                    .store(1.0_f32.to_bits(), Ordering::Relaxed);
                            } else {
                                av.bypassed_for_dop = false;
                            }
                            status.auto_volume = av.clone();
                            status.signal_path = build_signal_path(
                                &new_pipe.props,
                                new_pipe.output_rate,
                                new_pipe.output_channels,
                                &new_pipe.device_name,
                                new_pipe.sample_format,
                                config,
                                &av,
                            );
                        }
                        let mut s = shared.write();
                        s.track_info = Some(info);
                        s.status = status;
                        *pipeline = Some(new_pipe);
                    }
                } else {
                    // Refresh signal path description dynamically for EQ / headroom / auto-volume changes
                    let mut s = shared.write();
                    if config.auto_volume.mode == AutoVolumeMode::Off {
                        s.status.auto_volume = AutoVolumeStatus::default();
                    }
                    let av = s.status.auto_volume.clone();
                    s.status.signal_path = build_signal_path(
                        &pipe.props,
                        pipe.output_rate,
                        pipe.output_channels,
                        &pipe.device_name,
                        pipe.sample_format,
                        config,
                        &av,
                    );
                }
            }
        }

        AudioCommand::RefreshDevices => {
            let devs = enumerate_output_devices();
            shared.write().devices = devs;
        }
    }
}

fn open_and_start_pipeline(
    path: &Path,
    start_at: f64,
    autoplay: bool,
    config: &AppConfig,
    ctrl: &Arc<CallbackControl>,
) -> Result<(ActivePipeline, TrackInfo, PlaybackStatus)> {
    let mut track_info = probe_track_info(path);
    let mut decoder = open_decoder(path, &config.dsd)?;
    let props = decoder.properties().clone();

    if start_at > 0.05 {
        let _ = decoder.seek(start_at);
    }

    // Reconcile TrackInfo from exact decoder stream properties
    track_info.sample_rate = props.source_sample_rate;
    track_info.bit_depth = props.bit_depth;
    track_info.channels = props.channels;
    track_info.channel_labels = props.channel_labels.clone();
    track_info.format = props.format_name.clone();
    track_info.codec = props.codec_name.clone();
    track_info.is_dsd = props.is_dsd;
    track_info.dsd_rate = props.dsd_rate;
    track_info.decoder_backend = props.backend.to_string();
    if props.duration > 0.0 {
        track_info.duration = props.duration;
    }

    // Compute desired output rate
    let desired_rate = if props.is_dop {
        props.decoded_sample_rate // DoP rate is strictly dsd_rate / 16
    } else {
        config.upsample_mode.resolve_rate(props.decoded_sample_rate)
    };

    let device = select_cpal_device(&config.selected_device)?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| "Default Audio Output".to_string());

    let chosen = choose_output_config(
        &device,
        desired_rate,
        props.channels,
        config.multichannel.downmix_mode,
        props.is_dop,
    )?;

    let output_rate = chosen.config.sample_rate.0;
    let output_channels = chosen.config.channels;

    // Create streaming upsampler/resampler whenever decoded rate differs from DAC rate (and not DoP)
    let upsampler = if !props.is_dop && output_rate != props.decoded_sample_rate {
        Some(StreamingUpsampler::new(
            props.decoded_sample_rate,
            output_rate,
            props.channels as usize,
            config.resampler_quality,
            config.filter_rolloff,
        )?)
    } else {
        None
    };

    // Size ring buffer for ~0.5 seconds of output audio (smooth playback + instant seek)
    let ring_samples = ((output_rate as usize) * (output_channels as usize) / 2).max(16_384);
    let ring = Arc::new(AudioRingBuffer::new(ring_samples));

    ctrl.eof_reached.store(false, Ordering::SeqCst);
    ctrl.track_finished.store(false, Ordering::SeqCst);
    ctrl.played_frames.store(0, Ordering::SeqCst);
    ctrl.dop_mode.store(props.is_dop, Ordering::SeqCst);
    ctrl.playing.store(autoplay, Ordering::SeqCst);

    let stream = build_cpal_stream(&device, &chosen, Arc::clone(&ring), Arc::clone(ctrl))?;

    let initial_av = AutoVolumeStatus {
        active: false,
        analyzing: false,
        mode: config.auto_volume.mode,
        applied_gain_db: 0.0,
        measured_lufs: None,
        peak_dbfs: None,
        peak_limited: false,
        bypassed_for_dop: props.is_dop && config.auto_volume.mode != AutoVolumeMode::Off,
        source_label: if props.is_dop && config.auto_volume.mode != AutoVolumeMode::Off {
            "Bypassed (DoP)".to_string()
        } else {
            config.auto_volume.mode.short_label().to_string()
        },
    };

    let signal_path = build_signal_path(
        &props,
        output_rate,
        output_channels,
        &device_name,
        chosen.sample_format,
        config,
        &initial_av,
    );

    let upsampled = !props.is_dop && output_rate > props.decoded_sample_rate;
    let downmixed = !props.is_dop && output_channels < props.channels;

    let fmt_label = match chosen.sample_format {
        SampleFormat::F32 => "32-bit float",
        SampleFormat::I32 => "32-bit int",
        SampleFormat::I16 => "16-bit int",
        _ => "PCM",
    };

    let status = PlaybackStatus {
        state: if autoplay {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        },
        position: start_at,
        duration: track_info.duration,
        source_sample_rate: props.source_sample_rate,
        output_sample_rate: output_rate,
        source_channels: props.channels,
        output_channels,
        output_format_label: fmt_label.to_string(),
        output_device_name: device_name.clone(),
        is_dsd: props.is_dsd,
        is_dop: props.is_dop,
        upsampled,
        downmixed,
        auto_volume: initial_av,
        error_message: None,
        signal_path,
    };

    let eq = TenBandEq::new(output_rate, props.channels as usize, &config.eq);

    let pipe = ActivePipeline {
        path: path.to_path_buf(),
        decoder: Some(decoder),
        props,
        upsampler,
        eq,
        router: MultiChannelRouter::new(),
        ring,
        _stream: stream,
        output_rate,
        output_channels,
        device_name,
        sample_format: chosen.sample_format,
        seek_base_secs: start_at,
        pending_interleaved: Vec::new(),
        pending_offset: 0,
    };

    Ok((pipe, track_info, status))
}

    fn build_signal_path(
        props: &StreamProperties,
        output_rate: u32,
        output_channels: u16,
        device_name: &str,
        sample_format: SampleFormat,
        config: &AppConfig,
        auto_vol: &AutoVolumeStatus,
    ) -> SignalPath {
        let fmt_str = match sample_format {
            SampleFormat::F32 => "32-bit float",
            SampleFormat::I32 => "32-bit int",
            SampleFormat::I16 => "16-bit int",
            _ => "PCM",
        };
        super::build_signal_path_str(
            props,
            output_rate,
            output_channels,
            device_name,
            fmt_str,
            config,
            auto_vol,
        )
    }
}

#[cfg(not(target_os = "android"))]
pub use desktop::*;
