use std::f64::consts::PI;

use anyhow::Result;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::audio::ringbuf::VISUALIZER_FFT_SIZE;
use crate::config::{
    DownmixMode, EqSettings, FilterRolloff, MultiChannelSettings, ResamplerQuality, EQ_FREQUENCIES,
};

// ── 1. High-Precision Streaming Upsampler / Resampler (64-bit float internal) ──

pub struct StreamingUpsampler {
    resampler: SincFixedIn<f64>,
    channels: usize,
    chunk_size: usize,
    sinc_half_len: usize,
    ratio: f64,
    initial_delay_to_skip: usize,
    total_input_frames: u64,
    total_output_frames: u64,
    input_buffers: Vec<Vec<f64>>,
}

impl StreamingUpsampler {
    pub fn new(
        input_rate: u32,
        output_rate: u32,
        channels: usize,
        quality: ResamplerQuality,
        rolloff: FilterRolloff,
    ) -> Result<Self> {
        let ratio = output_rate as f64 / input_rate as f64;
        let chunk_size = 1024;
        let sinc_len = quality.sinc_len();

        let (interp, window) = match quality {
            ResamplerQuality::Fast => (SincInterpolationType::Linear, WindowFunction::Blackman),
            ResamplerQuality::High => (SincInterpolationType::Linear, WindowFunction::Blackman2),
            ResamplerQuality::Audiophile | ResamplerQuality::Reference => (
                SincInterpolationType::Cubic,
                WindowFunction::BlackmanHarris2,
            ),
        };

        let params = SincInterpolationParameters {
            sinc_len,
            f_cutoff: rolloff.f_cutoff(),
            interpolation: interp,
            oversampling_factor: quality.oversampling_factor(),
            window,
        };

        let resampler = SincFixedIn::<f64>::new(ratio, 2.0, params, chunk_size, channels)?;
        let initial_delay_to_skip = resampler.output_delay();

        Ok(Self {
            resampler,
            channels,
            chunk_size,
            sinc_half_len: sinc_len / 2,
            ratio,
            initial_delay_to_skip,
            total_input_frames: 0,
            total_output_frames: 0,
            input_buffers: vec![Vec::with_capacity(chunk_size * 2); channels],
        })
    }

    pub fn reset(&mut self) {
        self.resampler.reset();
        self.initial_delay_to_skip = self.resampler.output_delay();
        self.total_input_frames = 0;
        self.total_output_frames = 0;
        for buf in &mut self.input_buffers {
            buf.clear();
        }
    }

    /// Push planar `f32` audio and receive any complete resampled planar `f32` frames.
    pub fn process(&mut self, input: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        if input.is_empty() || input[0].is_empty() {
            return Ok(vec![Vec::new(); self.channels]);
        }

        self.total_input_frames += input[0].len() as u64;
        for ch in 0..self.channels {
            if ch < input.len() {
                self.input_buffers[ch].extend(input[ch].iter().map(|&s| s as f64));
            }
        }

        let mut output_all = vec![Vec::new(); self.channels];

        while self.input_buffers[0].len() >= self.chunk_size {
            let chunk: Vec<Vec<f64>> = self
                .input_buffers
                .iter_mut()
                .map(|buf| buf.drain(..self.chunk_size).collect())
                .collect();

            let resampled = self.resampler.process(&chunk, None)?;
            let got = resampled[0].len();
            let skip = self.initial_delay_to_skip.min(got);
            self.initial_delay_to_skip -= skip;
            self.total_output_frames += (got - skip) as u64;

            for ch in 0..self.channels {
                output_all[ch].extend(resampled[ch][skip..].iter().map(|&s| s as f32));
            }
        }

        Ok(output_all)
    }

    /// Flush remaining buffered samples and internal sinc filter group delay at EOF,
    /// returning sample-accurate tail frames.
    pub fn flush(&mut self) -> Result<Vec<Vec<f32>>> {
        let expected_total = (self.total_input_frames as f64 * self.ratio).round() as u64;
        let needed = expected_total.saturating_sub(self.total_output_frames) as usize;
        if needed == 0 {
            return Ok(vec![Vec::new(); self.channels]);
        }

        let mut output_all = vec![Vec::new(); self.channels];

        // Pad remaining input with enough zeros to drain filter delay completely
        let pad_zeros = self.sinc_half_len + self.chunk_size;
        for buf in &mut self.input_buffers {
            buf.resize(buf.len() + pad_zeros, 0.0);
        }

        while output_all[0].len() < needed && self.input_buffers[0].len() >= self.chunk_size {
            let chunk: Vec<Vec<f64>> = self
                .input_buffers
                .iter_mut()
                .map(|buf| buf.drain(..self.chunk_size).collect())
                .collect();
            let resampled = self.resampler.process(&chunk, None)?;
            let got = resampled[0].len();
            let skip = self.initial_delay_to_skip.min(got);
            self.initial_delay_to_skip -= skip;
            for ch in 0..self.channels {
                output_all[ch].extend(resampled[ch][skip..].iter().map(|&s| s as f32));
            }
        }

        for (out_ch, in_buf) in output_all.iter_mut().zip(self.input_buffers.iter_mut()) {
            out_ch.truncate(needed);
            in_buf.clear();
        }
        self.total_output_frames += output_all[0].len() as u64;

        Ok(output_all)
    }
}

// ── 2. 10-Band Audiophile 64-bit Biquad Equalizer ────────────────────────────

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

impl Biquad {
    fn set_identity(&mut self) {
        self.b0 = 1.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.a1 = 0.0;
        self.a2 = 0.0;
    }

    fn set_peaking(&mut self, fs: f64, freq: f64, q: f64, gain_db: f64) {
        if gain_db.abs() < 0.01 || freq >= fs * 0.48 {
            self.set_identity();
            return;
        }
        let a = 10.0f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_low_shelf(&mut self, fs: f64, freq: f64, gain_db: f64) {
        if gain_db.abs() < 0.01 || freq >= fs * 0.48 {
            self.set_identity();
            return;
        }
        let a = 10.0f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() * 0.5 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_high_shelf(&mut self, fs: f64, freq: f64, gain_db: f64) {
        if gain_db.abs() < 0.01 || freq >= fs * 0.48 {
            self.set_identity();
            return;
        }
        let a = 10.0f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() * 0.5 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline(always)]
    fn process(&mut self, x: f64) -> f64 {
        // Direct Form II Transposed for minimal quantization noise
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

pub struct TenBandEq {
    sample_rate: u32,
    channels: usize,
    filters: Vec<[Biquad; 10]>,
    preamp_lin: f64,
    last_settings: EqSettings,
}

impl TenBandEq {
    pub fn new(sample_rate: u32, channels: usize, settings: &EqSettings) -> Self {
        let mut eq = Self {
            sample_rate,
            channels,
            filters: vec![[Biquad::default(); 10]; channels],
            preamp_lin: 1.0,
            last_settings: EqSettings::default(),
        };
        eq.update(sample_rate, channels, settings);
        eq
    }

    pub fn update(&mut self, sample_rate: u32, channels: usize, settings: &EqSettings) {
        if self.channels != channels {
            self.channels = channels;
            self.filters = vec![[Biquad::default(); 10]; channels];
        } else if self.sample_rate == sample_rate && self.last_settings == *settings {
            return;
        }
        self.sample_rate = sample_rate;
        self.last_settings = settings.clone();
        self.preamp_lin = 10.0f64.powf(settings.preamp_db as f64 / 20.0);

        let fs = sample_rate.max(8000) as f64;
        for ch_filters in &mut self.filters {
            for (idx, bq) in ch_filters.iter_mut().enumerate() {
                let freq = EQ_FREQUENCIES[idx];
                let gain = settings.bands_db[idx] as f64;
                match idx {
                    0 => bq.set_low_shelf(fs, freq, gain),
                    9 => bq.set_high_shelf(fs, freq, gain),
                    _ => bq.set_peaking(fs, freq, 1.414, gain),
                }
            }
        }
    }

    pub fn reset(&mut self) {
        for ch_filters in &mut self.filters {
            for bq in ch_filters.iter_mut() {
                bq.reset();
            }
        }
    }

    pub fn process_in_place(&mut self, planar: &mut [Vec<f32>], settings: &EqSettings) {
        if !settings.enabled {
            return;
        }
        let preamp = self.preamp_lin;
        for (channel, filters) in planar.iter_mut().zip(self.filters.iter_mut()) {
            for sample in channel.iter_mut() {
                let mut x = (*sample as f64) * preamp;
                for bq in filters.iter_mut() {
                    x = bq.process(x);
                }
                *sample = x as f32;
            }
        }
    }
}

// ── 3. Multi-Channel Matrix Router, ITU-R BS.775 Downmixer & Crossfeed ───────

#[derive(Debug, Clone, Default)]
pub struct ChannelMuteSolo {
    pub muted: [bool; 8],
    pub solo: [bool; 8],
}

#[derive(Default)]
pub struct MultiChannelRouter {
    /// Simple 1-pole low-pass crossfeed filter state for Headphone Spatial mode
    xf_l: f32,
    xf_r: f32,
}

impl MultiChannelRouter {
    pub fn new() -> Self {
        Self {
            xf_l: 0.0,
            xf_r: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.xf_l = 0.0;
        self.xf_r = 0.0;
    }

    /// Apply per-channel mute/solo directly to source planar buffers (before metering/downmix).
    pub fn apply_mute_solo(planar: &mut [Vec<f32>], ms: &ChannelMuteSolo) {
        let any_solo = ms.solo.iter().take(planar.len().min(8)).any(|&s| s);
        for (ch, buf) in planar.iter_mut().enumerate() {
            let silenced = if ch < 8 {
                if any_solo {
                    !ms.solo[ch]
                } else {
                    ms.muted[ch]
                }
            } else {
                any_solo
            };
            if silenced {
                buf.fill(0.0);
            }
        }
    }

    /// Route/downmix planar source channels (`in_ch × frames`) into interleaved `f32`
    /// output samples matching `out_channels`, with headroom & optional soft clipping.
    #[allow(clippy::needless_range_loop)]
    pub fn mix_to_interleaved(
        &mut self,
        planar: &[Vec<f32>],
        out_channels: usize,
        settings: &MultiChannelSettings,
        headroom_db: f32,
        soft_clip: bool,
        bypass_gain_for_dop: bool,
    ) -> Vec<f32> {
        let in_ch = planar.len();
        if in_ch == 0 || out_channels == 0 {
            return Vec::new();
        }
        let frames = planar[0].len();
        if frames == 0 {
            return Vec::new();
        }

        let mut interleaved = vec![0.0f32; frames * out_channels];

        // Bit-perfect DoP passthrough: never scale, downmix, or clip!
        if bypass_gain_for_dop {
            for f in 0..frames {
                let base = f * out_channels;
                for c in 0..out_channels {
                    interleaved[base + c] = if c < in_ch { planar[c][f] } else { 0.0 };
                }
            }
            return interleaved;
        }

        let headroom = db_to_lin(headroom_db.clamp(-12.0, 0.0));
        let force_stereo = matches!(
            settings.downmix_mode,
            DownmixMode::StereoItu | DownmixMode::HeadphoneSpatial
        );
        let spatial = settings.downmix_mode == DownmixMode::HeadphoneSpatial;

        let c_gain = db_to_lin(settings.center_gain_db);
        let s_gain = db_to_lin(settings.surround_gain_db);
        let lfe_gain = if settings.include_lfe_in_downmix {
            db_to_lin(settings.lfe_gain_db)
        } else {
            0.0
        };

        if (out_channels == 2 && (in_ch > 2 || force_stereo)) || (out_channels == 1 && in_ch > 1) {
            // Downmix to Stereo (2.0) or Mono (1.0) using ITU-R BS.775-1 matrix
            let norm = match in_ch {
                1 => 1.0,
                2 => 1.0,
                3 => 1.0 / (1.0 + c_gain),
                4 => 1.0 / (1.0 + s_gain),
                5 => 1.0 / (1.0 + c_gain + s_gain),
                6 => 1.0 / (1.0 + c_gain + s_gain + lfe_gain),
                7 => 1.0 / (1.0 + c_gain + 1.5 * s_gain + lfe_gain),
                _ => 1.0 / (1.0 + c_gain + 1.414 * s_gain + lfe_gain),
            };

            let xf_amt = if spatial {
                settings.crossfeed_amount.clamp(0.0, 0.5)
            } else {
                0.0
            };

            for f in 0..frames {
                let (mut l, mut r) = match in_ch {
                    1 => {
                        let m = planar[0][f];
                        (m, m)
                    }
                    2 => (planar[0][f], planar[1][f]),
                    3 => {
                        // FL, FR, FC
                        let fc = planar[2][f] * c_gain;
                        (planar[0][f] + fc, planar[1][f] + fc)
                    }
                    4 => {
                        // FL, FR, BL/SL, BR/SR
                        let sl = planar[2][f] * s_gain;
                        let sr = planar[3][f] * s_gain;
                        (planar[0][f] + sl, planar[1][f] + sr)
                    }
                    5 => {
                        // FL, FR, FC, SL, SR
                        let fc = planar[2][f] * c_gain;
                        let sl = planar[3][f] * s_gain;
                        let sr = planar[4][f] * s_gain;
                        (planar[0][f] + fc + sl, planar[1][f] + fc + sr)
                    }
                    6 => {
                        // Standard 5.1: FL(0), FR(1), FC(2), LFE(3), SL/BL(4), SR/BR(5)
                        let fc = planar[2][f] * c_gain;
                        let lfe = planar[3][f] * lfe_gain;
                        let sl = planar[4][f] * s_gain;
                        let sr = planar[5][f] * s_gain;
                        if spatial {
                            // Subtle spacious rear decorrelation for headphones
                            (
                                planar[0][f] + fc + lfe + sl - 0.12 * sr,
                                planar[1][f] + fc + lfe + sr - 0.12 * sl,
                            )
                        } else {
                            (planar[0][f] + fc + lfe + sl, planar[1][f] + fc + lfe + sr)
                        }
                    }
                    7 => {
                        // 6.1: FL(0), FR(1), FC(2), LFE(3), BC(4), SL(5), SR(6)
                        let fc = planar[2][f] * c_gain;
                        let lfe = planar[3][f] * lfe_gain;
                        let bc = planar[4][f] * s_gain * 0.707;
                        let sl = planar[5][f] * s_gain;
                        let sr = planar[6][f] * s_gain;
                        (
                            planar[0][f] + fc + lfe + sl + bc,
                            planar[1][f] + fc + lfe + sr + bc,
                        )
                    }
                    _ => {
                        // 7.1+: FL(0), FR(1), FC(2), LFE(3), BL(4), BR(5), SL(6), SR(7)
                        let fc = planar[2][f] * c_gain;
                        let lfe = planar[3][f] * lfe_gain;
                        let bl = planar[4][f] * s_gain * 0.707;
                        let br = planar[5][f] * s_gain * 0.707;
                        let sl = planar[6][f] * s_gain * 0.707;
                        let sr = planar[7][f] * s_gain * 0.707;
                        let mut extra_l = 0.0f32;
                        let mut extra_r = 0.0f32;
                        for ch in 8..in_ch {
                            let s = planar[ch][f] * 0.35;
                            if ch % 2 == 0 {
                                extra_l += s;
                            } else {
                                extra_r += s;
                            }
                        }
                        if spatial {
                            (
                                planar[0][f] + fc + lfe + sl + bl + extra_l - 0.14 * (sr + br),
                                planar[1][f] + fc + lfe + sr + br + extra_r - 0.14 * (sl + bl),
                            )
                        } else {
                            (
                                planar[0][f] + fc + lfe + sl + bl + extra_l,
                                planar[1][f] + fc + lfe + sr + br + extra_r,
                            )
                        }
                    }
                };

                l *= norm;
                r *= norm;

                // Acoustic Headphone Crossfeed (low-pass filtered interaural bleed)
                if xf_amt > 0.001 {
                    let alpha = 0.15f32; // warm low-frequency acoustic shadow
                    self.xf_l += alpha * (l - self.xf_l);
                    self.xf_r += alpha * (r - self.xf_r);
                    let inv = 1.0 / (1.0 + xf_amt);
                    l = (l + xf_amt * self.xf_r) * inv;
                    r = (r + xf_amt * self.xf_l) * inv;
                }

                l = apply_headroom_and_clip(l * headroom, soft_clip);
                r = apply_headroom_and_clip(r * headroom, soft_clip);

                let base = f * out_channels;
                if out_channels == 1 {
                    interleaved[base] = 0.5 * (l + r);
                } else {
                    interleaved[base] = l;
                    interleaved[base + 1] = r;
                }
            }
        } else if out_channels == 6 && in_ch == 8 {
            // 7.1 -> 5.1 fold-down (combines Back + Side surrounds cleanly)
            for f in 0..frames {
                let base = f * 6;
                interleaved[base] = apply_headroom_and_clip(planar[0][f] * headroom, soft_clip);
                interleaved[base + 1] =
                    apply_headroom_and_clip(planar[1][f] * headroom, soft_clip);
                interleaved[base + 2] =
                    apply_headroom_and_clip(planar[2][f] * headroom, soft_clip);
                interleaved[base + 3] =
                    apply_headroom_and_clip(planar[3][f] * headroom, soft_clip);
                let sl = (planar[4][f] + planar[6][f]) * std::f32::consts::FRAC_1_SQRT_2;
                let sr = (planar[5][f] + planar[7][f]) * std::f32::consts::FRAC_1_SQRT_2;
                interleaved[base + 4] = apply_headroom_and_clip(sl * headroom, soft_clip);
                interleaved[base + 5] = apply_headroom_and_clip(sr * headroom, soft_clip);
            }
        } else {
            // Native 1:1 multi-channel passthrough (or mono->stereo expansion)
            for f in 0..frames {
                let base = f * out_channels;
                if in_ch == 1 && out_channels >= 2 {
                    let s = apply_headroom_and_clip(planar[0][f] * headroom, soft_clip);
                    interleaved[base] = s;
                    interleaved[base + 1] = s;
                } else {
                    for c in 0..out_channels {
                        let raw = if c < in_ch { planar[c][f] } else { 0.0 };
                        interleaved[base + c] =
                            apply_headroom_and_clip(raw * headroom, soft_clip);
                    }
                }
            }
        }

        interleaved
    }
}

#[inline(always)]
fn db_to_lin(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

#[inline(always)]
fn apply_headroom_and_clip(x: f32, soft_clip: bool) -> f32 {
    if soft_clip {
        // Transparent linear region up to 0.96, gentle tanh knee above to avoid harsh digital clipping
        let ax = x.abs();
        if ax <= 0.96 {
            x
        } else {
            let excess = (ax - 0.96) / 0.04;
            let limited = 0.96 + 0.04 * excess.tanh();
            limited.copysign(x)
        }
    } else {
        x.clamp(-1.0, 1.0)
    }
}

// ── 4. Real-Time 48-Band FFT Spectrum Analyzer (Pure Rust) ───────────────────

pub const SPECTRUM_BANDS: usize = 48;

pub struct SpectrumAnalyzer {
    pub bands: [f32; SPECTRUM_BANDS],
    pub peaks: [f32; SPECTRUM_BANDS],
    window: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let n = VISUALIZER_FFT_SIZE;
        let window: Vec<f32> = (0..n)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f64 / (n - 1) as f64).cos()) as f32)
            .collect();
        Self {
            bands: [0.0; SPECTRUM_BANDS],
            peaks: [0.0; SPECTRUM_BANDS],
            window,
        }
    }

    pub fn decay_only(&mut self) {
        for i in 0..SPECTRUM_BANDS {
            self.bands[i] *= 0.85;
            self.peaks[i] = (self.peaks[i] - 0.015).max(0.0);
        }
    }

    pub fn analyze(&mut self, samples: &[f32; VISUALIZER_FFT_SIZE], sample_rate: u32) {
        let n = VISUALIZER_FFT_SIZE;
        let mut re = vec![0.0f32; n];
        let mut im = vec![0.0f32; n];
        for i in 0..n {
            re[i] = samples[i] * self.window[i];
        }

        fft_radix2(&mut re, &mut im);

        let nyquist = (sample_rate.max(8000) as f32) * 0.5;
        let min_freq = 24.0f32;
        let max_freq = nyquist.clamp(4000.0, 22_000.0);
        let log_min = min_freq.ln();
        let log_max = max_freq.ln();
        let half_n = n / 2;
        let bin_hz = (sample_rate.max(8000) as f32) / (n as f32);
        let scale = 2.0 / (n as f32);

        for b in 0..SPECTRUM_BANDS {
            let f0 = (log_min + (log_max - log_min) * (b as f32 / SPECTRUM_BANDS as f32)).exp();
            let f1 = (log_min + (log_max - log_min) * ((b + 1) as f32 / SPECTRUM_BANDS as f32))
                .exp();

            let bin0 = ((f0 / bin_hz).floor() as usize).clamp(1, half_n - 1);
            let bin1 = ((f1 / bin_hz).ceil() as usize).clamp(bin0 + 1, half_n);

            let mut max_mag = 0.0f32;
            for k in bin0..bin1 {
                let mag = (re[k] * re[k] + im[k] * im[k]).sqrt() * scale;
                if mag > max_mag {
                    max_mag = mag;
                }
            }

            // Convert magnitude to normalized 0..1 bar height across -68 dB .. -2 dB
            let db = 20.0 * (max_mag.max(1e-6)).log10();
            let norm = ((db + 68.0) / 66.0).clamp(0.0, 1.0);

            // Smooth attack / release ballistics
            if norm > self.bands[b] {
                self.bands[b] = self.bands[b] * 0.35 + norm * 0.65;
            } else {
                self.bands[b] = self.bands[b] * 0.80 + norm * 0.20;
            }

            if self.bands[b] > self.peaks[b] {
                self.peaks[b] = self.bands[b];
            } else {
                self.peaks[b] = (self.peaks[b] - 0.012).max(self.bands[b]);
            }
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// In-place iterative Cooley-Tukey radix-2 FFT (`re.len()` must be power of two).
fn fft_radix2(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let half = len >> 1;
        let angle = -2.0 * std::f32::consts::PI / (len as f32);
        let w_re = angle.cos();
        let w_im = angle.sin();
        let mut i = 0;
        while i < n {
            let mut cur_re = 1.0f32;
            let mut cur_im = 0.0f32;
            for k in 0..half {
                let u_re = re[i + k];
                let u_im = im[i + k];
                let t_re = re[i + k + half] * cur_re - im[i + k + half] * cur_im;
                let t_im = re[i + k + half] * cur_im + im[i + k + half] * cur_re;
                re[i + k] = u_re + t_re;
                im[i + k] = u_im + t_im;
                re[i + k + half] = u_re - t_re;
                im[i + k + half] = u_im - t_im;

                let next_re = cur_re * w_re - cur_im * w_im;
                cur_im = cur_re * w_im + cur_im * w_re;
                cur_re = next_re;
            }
            i += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_upsampler_44100_to_176400_preserves_tone_and_ratio() {
        let mut up = StreamingUpsampler::new(
            44_100,
            176_400,
            2,
            ResamplerQuality::Audiophile,
            FilterRolloff::Sharp,
        )
        .unwrap();

        let frames = 4096;
        let sine: Vec<f32> = (0..frames)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin() * 0.5)
            .collect();

        let mut out_l = Vec::new();
        for chunk in sine.chunks(512) {
            let res = up.process(&[chunk.to_vec(), chunk.to_vec()]).unwrap();
            out_l.extend_from_slice(&res[0]);
        }
        let tail = up.flush().unwrap();
        out_l.extend_from_slice(&tail[0]);

        // 4x upsampling should produce ~4x frames
        let expected = frames * 4;
        assert!(
            (out_l.len() as isize - expected as isize).abs() < 64,
            "Got {} frames, expected ~{}",
            out_l.len(),
            expected
        );
        let peak = out_l.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!((peak - 0.5).abs() < 0.05, "Unexpected peak {}", peak);
    }

    #[test]
    fn itu_51_downmix_preserves_center_and_surround_without_clipping() {
        let mut router = MultiChannelRouter::new();
        let settings = MultiChannelSettings {
            downmix_mode: DownmixMode::StereoItu,
            include_lfe_in_downmix: true,
            ..Default::default()
        };

        // 5.1 signal with strong Center (ch2) and Surround Left (ch4)
        let planar = vec![
            vec![0.0f32; 64], // FL
            vec![0.0f32; 64], // FR
            vec![0.8f32; 64], // FC
            vec![0.5f32; 64], // LFE
            vec![0.6f32; 64], // SL
            vec![0.0f32; 64], // SR
        ];

        let stereo = router.mix_to_interleaved(&planar, 2, &settings, 0.0, false, false);
        assert_eq!(stereo.len(), 128);
        let l = stereo[0];
        let r = stereo[1];
        assert!(l > r, "Left should be stronger than right due to SL");
        assert!(r > 0.15, "Right should still carry Center + LFE");
        assert!(l <= 1.0 && r <= 1.0, "Normalized mix must never clip");
    }

    #[test]
    fn ten_band_eq_boosts_target_band_when_enabled() {
        let mut bands = [0.0; 10];
        bands[5] = 6.0; // +6 dB at 1 kHz
        let settings = EqSettings {
            enabled: true,
            bands_db: bands,
            ..Default::default()
        };

        let mut eq = TenBandEq::new(48_000, 1, &settings);
        let signal: Vec<f32> = (0..4096)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin() * 0.25)
            .collect();
        let mut planar = vec![signal.clone()];
        eq.process_in_place(&mut planar, &settings);

        let orig_rms: f32 =
            (signal[1024..].iter().map(|s| s * s).sum::<f32>() / 3072.0).sqrt();
        let eq_rms: f32 =
            (planar[0][1024..].iter().map(|s| s * s).sum::<f32>() / 3072.0).sqrt();
        let gain_db = 20.0 * (eq_rms / orig_rms).log10();
        assert!(
            (gain_db - 6.0).abs() < 0.8,
            "Expected ~+6 dB at 1 kHz, got {:.2} dB",
            gain_db
        );
    }
}
