use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::audio::decoder::{StreamProperties, StreamingDecoder, TrackInfo};
use crate::audio::dsd::default_channel_labels;
use crate::config::DsdSettings;

pub struct DemoTrackSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub artist: &'static str,
    pub album: &'static str,
    pub catalog_no: &'static str,
    pub duration: f64,
    pub sample_rate: u32,
    pub bit_depth: u16,
    pub channels: u16,
    pub format: &'static str,
    pub codec: &'static str,
    pub is_dsd: bool,
    pub base_hz: f64,
    pub cover_variant: u8,
    pub synced_lyrics: &'static str,
}

pub const DEMO_CATALOG: &[DemoTrackSpec] = &[
    DemoTrackSpec {
        id: "nocturne-in-amber",
        title: "Nocturne in Amber (Studio Master)",
        artist: "Klara Lindqvist Quartet",
        album: "Nordic Sessions Vol. IV",
        catalog_no: "SNR-041",
        duration: 278.0,
        sample_rate: 192_000,
        bit_depth: 24,
        channels: 2,
        format: "FLAC",
        codec: "FLAC",
        is_dsd: false,
        base_hz: 220.0, // A3 warm minor 9th voicing
        cover_variant: 0,
        synced_lyrics: "\
[00:00.00] (Warm tape room tone — Studio A, Stockholm)
[00:04.50] Still air across the cedar floor
[00:09.80] Brass light leans through the northern pane
[00:15.20] Every wire holds the quiet hum
[00:21.00] Before the bow begins the phrase
[00:27.40] Amber shadows cross the metronome
[00:33.10] Slow breath, silver overtone
[00:39.00] Let the room decay into the dark
[00:45.60] Where every harmonic stands alone
[00:52.20] (Instrumental interlude — upright bass & brushed snare)
[01:04.00] Soft glass ringing on the seventh chord
[01:10.50] Night rain settles on the iron roof
[01:17.20] Measure by measure, clear and unhurried
[01:24.00] Nothing hurried, nothing lost in noise
[01:31.50] Hold the last note till the room goes still
",
    },
    DemoTrackSpec {
        id: "glass-cathedral-51",
        title: "Glass Cathedral (Live 5.1 SACD)",
        artist: "Sinfonia Helvetia",
        album: "Basel Acoustic Hall",
        catalog_no: "SNR-128",
        duration: 374.0,
        sample_rate: 5_644_800,
        bit_depth: 1,
        channels: 6,
        format: "DSF",
        codec: "DSD128",
        is_dsd: true,
        base_hz: 196.0, // G3 open cathedral fifths
        cover_variant: 1,
        synced_lyrics: "\
[00:00.00] (5.1 Surround Hall Ambience — Basel Kunsthalle)
[00:05.00] Stone arches lift the first low fifth
[00:11.40] Center strings answer through the nave
[00:18.20] Rear galleries return the bloom
[00:25.00] A slow tide breaking without haste
[00:32.50] Light fractures down the eastern vault
[00:39.80] Six discrete voices in the dark
[00:47.00] Sub-bass anchors the marble floor
[00:54.50] While upper harmonics float in air
[01:03.00] (Tutti swell — full 5.1 surround array)
[01:14.20] Quiet returns to every corner
[01:22.00] Leaving only resonant space
",
    },
    DemoTrackSpec {
        id: "midnight-transit",
        title: "Midnight Transit",
        artist: "Kenji Takahashi",
        album: "Tokyo Tape Studies",
        catalog_no: "SNR-352",
        duration: 232.0,
        sample_rate: 352_800,
        bit_depth: 32,
        channels: 2,
        format: "WAV",
        codec: "PCM 32f DXD",
        is_dsd: false,
        base_hz: 261.63, // C4 electric piano maj7
        cover_variant: 2,
        synced_lyrics: "\
[00:00.00] (DXD 352.8 kHz Direct Master)
[00:04.00] Sodium lamps along the elevated rail
[00:09.50] Wet asphalt mirrors every passing car
[00:15.00] Electric piano through a spring reverb
[00:20.80] Counting stations in the rain
[00:26.50] Neon fades behind the tunnel wall
[00:32.40] Clean silence in the space between beats
[00:38.80] Next stop Shinjuku, quarter past one
[00:45.00] Carrying the tape reel home
",
    },
    DemoTrackSpec {
        id: "concrete-and-brass-71",
        title: "Concrete & Brass (7.1 Spatial Mix)",
        artist: "Atelier 04 Ensemble",
        album: "Zürich Spatial Works",
        catalog_no: "SNR-710",
        duration: 326.0,
        sample_rate: 96_000,
        bit_depth: 24,
        channels: 8,
        format: "FLAC",
        codec: "FLAC",
        is_dsd: false,
        base_hz: 174.61, // F3 modernist brass cluster
        cover_variant: 3,
        synced_lyrics: "\
[00:00.00] (7.1 Discrete Spatial Mix — Channels FL FR FC LFE BL BR SL SR)
[00:05.20] Geometric lines in raw concrete
[00:11.00] Flugelhorn calls from front left stage
[00:17.40] Trombone answers along the side wall
[00:23.80] Eight channels locked to a single grid
[00:30.20] Precision pulse beneath the brass
[00:37.00] Try Solo (S) on FC or LFE in the Multi-Channel tab
[00:44.50] To inspect each isolated speaker stem
",
    },
    DemoTrackSpec {
        id: "rain-on-cedar",
        title: "Rain on Cedar",
        artist: "Maren Solberg",
        album: "Acoustic Sketches",
        catalog_no: "SNR-064",
        duration: 249.0,
        sample_rate: 2_822_400,
        bit_depth: 1,
        channels: 2,
        format: "DFF",
        codec: "DSD64",
        is_dsd: true,
        base_hz: 246.94, // B3 acoustic fingerstyle
        cover_variant: 4,
        synced_lyrics: "\
[00:00.00] (Direct-to-DSD64 Single Microphone Capture)
[00:04.20] Cedar body, nylon strings
[00:09.60] Raindrops tapping on the porch
[00:15.40] One ribbon mic, no compression
[00:21.20] Just timber, breath, and autumn air
[00:27.50] Evening slows along the fjord
[00:34.00] Soft harmonics fade to grey
",
    },
];

pub fn demo_path(id: &str) -> PathBuf {
    PathBuf::from(format!("demo://{}", id))
}

pub fn demo_spec_for_path(path: &Path) -> Option<&'static DemoTrackSpec> {
    let s = path.to_str()?;
    let id = s.strip_prefix("demo://")?;
    DEMO_CATALOG.iter().find(|t| t.id == id)
}

pub fn demo_track_info(spec: &'static DemoTrackSpec) -> TrackInfo {
    TrackInfo {
        title: spec.title.to_string(),
        artist: spec.artist.to_string(),
        album: format!("{} · {}", spec.album, spec.catalog_no),
        duration: spec.duration,
        sample_rate: spec.sample_rate,
        bit_depth: spec.bit_depth,
        channels: spec.channels,
        channel_labels: default_channel_labels(spec.channels),
        format: spec.format.to_string(),
        codec: spec.codec.to_string(),
        is_dsd: spec.is_dsd,
        dsd_rate: if spec.is_dsd {
            Some(spec.sample_rate)
        } else {
            None
        },
        decoder_backend: if spec.is_dsd {
            "Studio DSD Reference".to_string()
        } else {
            "Studio PCM Reference".to_string()
        },
        album_art: None,
        file_path: demo_path(spec.id),
    }
}

/// Generate a crisp 320x320 Swiss editorial / modernist album sleeve RGBA image for a showcase track.
pub fn generate_swiss_cover_rgba(variant: u8) -> (usize, usize, Vec<u8>) {
    let w = 320usize;
    let h = 320usize;
    let mut rgba = vec![0u8; w * h * 4];

    // Palette per sleeve: grounded in Swiss high-contrast carbon + warm paper + Braun orange / brass / emerald
    let (bg, shape1, shape2, rule) = match variant % 5 {
        0 => (
            [14u8, 14, 17],
            [232, 93, 42],  // Braun international orange
            [242, 240, 232], // gallery white
            [42, 42, 48],
        ),
        1 => (
            [12u8, 13, 16],
            [212, 180, 106], // brushed brass
            [36, 38, 46],
            [52, 50, 44],
        ),
        2 => (
            [11u8, 11, 14],
            [242, 240, 232],
            [232, 93, 42],
            [38, 38, 44],
        ),
        3 => (
            [14u8, 15, 18],
            [60, 200, 140], // precision emerald
            [212, 180, 106],
            [40, 44, 48],
        ),
        _ => (
            [15u8, 14, 14],
            [212, 180, 106],
            [232, 93, 42],
            [48, 44, 42],
        ),
    };

    for y in 0..h {
        for x in 0..w {
            let mut c = bg;
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;

            // Subtle Swiss 4x4 grid hairline rules
            if x % 80 == 0 || y % 80 == 0 {
                c = rule;
            }

            match variant % 5 {
                0 => {
                    // Minimalist Bauhaus concentric arcs + bold offset circle
                    let d = (fx - 0.56).hypot(fy - 0.52);
                    if (0.28..0.34).contains(&d) {
                        c = shape2;
                    } else if d < 0.19 {
                        c = shape1;
                    }
                    // Crisp horizontal architectural rule
                    if (248..=250).contains(&y) && (24..=296).contains(&x) {
                        c = shape2;
                    }
                }
                1 => {
                    // 5.1 Acoustic chamber vertical column bars (Swiss poster composition)
                    let cols = [0.16f32, 0.28, 0.42, 0.56, 0.70, 0.82];
                    let heights = [0.58f32, 0.74, 0.88, 0.46, 0.70, 0.54];
                    for (idx, (&cx, &ch)) in cols.iter().zip(heights.iter()).enumerate() {
                        if (fx - cx).abs() < 0.036 && fy > (0.90 - ch) && fy < 0.90 {
                            c = if idx == 2 { shape1 } else { shape2 };
                        }
                    }
                    let d = (fx - 0.42).hypot(fy - 0.24);
                    if d < 0.055 {
                        c = shape1;
                    }
                }
                2 => {
                    // Diagonal Swiss modernist split + precision disc
                    if fx + fy > 1.05 && fx + fy < 1.18 {
                        c = shape2;
                    }
                    let d = (fx - 0.36).hypot(fy - 0.42);
                    if d < 0.22 && d > 0.08 {
                        c = shape1;
                    }
                }
                3 => {
                    // 7.1 Spatial acoustic node grid (8 circular nodes around a ring)
                    let center_d = (fx - 0.5).hypot(fy - 0.5);
                    if (0.315..0.325).contains(&center_d) {
                        c = shape2;
                    }
                    for k in 0..8 {
                        let a = (k as f32) * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
                        let nx = 0.5 + 0.32 * a.cos();
                        let ny = 0.5 + 0.32 * a.sin();
                        if (fx - nx).hypot(fy - ny) < 0.042 {
                            c = if k == 0 || k == 4 { shape1 } else { shape2 };
                        }
                    }
                    if center_d < 0.06 {
                        c = shape1;
                    }
                }
                _ => {
                    // Warm harmonic wave ribbons (DSD direct capture)
                    let wave1 = 0.42 + 0.14 * (fx * std::f32::consts::TAU * 1.5).sin();
                    let wave2 = 0.56 + 0.10 * (fx * std::f32::consts::TAU * 2.5 + 1.1).sin();
                    if (fy - wave1).abs() < 0.022 {
                        c = shape1;
                    } else if (fy - wave2).abs() < 0.015 {
                        c = shape2;
                    }
                }
            }

            let idx = (y * w + x) * 4;
            rgba[idx] = c[0];
            rgba[idx + 1] = c[1];
            rgba[idx + 2] = c[2];
            rgba[idx + 3] = 255;
        }
    }

    (w, h, rgba)
}

/// Real-time musical studio reference synthesizer streaming through the exact same
/// Upsampler, 10-Band EQ, Multi-Channel Router, Spectrum Analyzer, and VU Meters.
pub struct DemoSynthDecoder {
    spec: &'static DemoTrackSpec,
    props: StreamProperties,
    current_frame: u64,
    total_frames: u64,
}

impl DemoSynthDecoder {
    pub fn open(path: &Path, dsd_settings: &DsdSettings) -> Result<Self> {
        let spec = demo_spec_for_path(path).ok_or_else(|| anyhow!("Unknown demo track"))?;
        let decoded_rate = if spec.is_dsd {
            dsd_settings.output_mode.target_pcm_rate().unwrap_or(176_400)
        } else {
            spec.sample_rate
        };

        let props = StreamProperties {
            source_sample_rate: spec.sample_rate,
            decoded_sample_rate: decoded_rate,
            channels: spec.channels,
            channel_labels: default_channel_labels(spec.channels),
            bit_depth: spec.bit_depth,
            duration: spec.duration,
            format_name: spec.format.to_string(),
            codec_name: spec.codec.to_string(),
            backend: if spec.is_dsd {
                "Studio DSD Reference"
            } else {
                "Studio PCM Reference"
            },
            is_dsd: spec.is_dsd,
            is_dop: false,
            dsd_rate: if spec.is_dsd {
                Some(spec.sample_rate)
            } else {
                None
            },
        };

        let total_frames = (spec.duration * decoded_rate as f64) as u64;
        Ok(Self {
            spec,
            props,
            current_frame: 0,
            total_frames,
        })
    }
}

impl StreamingDecoder for DemoSynthDecoder {
    fn properties(&self) -> &StreamProperties {
        &self.props
    }

    fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>> {
        if self.current_frame >= self.total_frames {
            return Ok(None);
        }

        let ch_count = self.props.channels as usize;
        let sr = self.props.decoded_sample_rate as f64;
        let frames = 2048usize.min((self.total_frames - self.current_frame) as usize);
        let mut planar = vec![Vec::with_capacity(frames); ch_count];

        // Gentle warm chord progression (4-bar harmonic cycle)
        let chord_ratios: [[f64; 4]; 4] = [
            [1.0, 1.2, 1.5, 1.875],     // min7 / warm triad
            [0.89, 1.125, 1.333, 1.68], // IV-9
            [0.75, 1.0, 1.25, 1.5],     // VI maj7
            [0.84, 1.06, 1.26, 1.58],   // V sus
        ];
        let base = self.spec.base_hz;

        for f in 0..frames {
            let idx = self.current_frame + f as u64;
            let t = idx as f64 / sr;
            let bar = ((t / 4.0).floor() as usize) % 4;
            let bar_pos = (t % 4.0) / 4.0;
            // Smooth acoustic attack/decay envelope per chord
            let env = (1.0 - (-bar_pos * 18.0).exp()) * (0.45 + 0.55 * (-bar_pos * 1.4).exp());
            let arp_pulse = 0.65 + 0.35 * (2.0 * PI * 0.5 * t).sin();
            let ratios = chord_ratios[bar];

            let root = (2.0 * PI * base * ratios[0] * t).sin();
            let third = (2.0 * PI * base * ratios[1] * t).sin();
            let fifth = (2.0 * PI * base * ratios[2] * t).sin();
            let seventh = (2.0 * PI * base * ratios[3] * t).sin();
            let overtone = (2.0 * PI * base * ratios[2] * 2.0 * t).sin() * 0.18;
            let sub = (2.0 * PI * (base * 0.5) * ratios[0] * t).sin();

            // Gentle L/R stereo shimmer
            let pan_l = 0.55 + 0.25 * (2.0 * PI * 0.125 * t).sin();
            let pan_r = 0.55 + 0.25 * (2.0 * PI * 0.125 * t + PI * 0.65).sin();

            for (ch, ch_buf) in planar.iter_mut().enumerate().take(ch_count) {
                let s = match ch {
                    0 => {
                        // FL: root + fifth + shimmer
                        (0.32 * root + 0.22 * fifth + 0.14 * seventh * pan_l + overtone) * env
                    }
                    1 => {
                        // FR: root + third + shimmer
                        (0.30 * root + 0.24 * third + 0.16 * seventh * pan_r + overtone * 0.9) * env
                    }
                    2 => {
                        // FC: focused center melody harmonic
                        (0.34 * third + 0.22 * fifth) * env * arp_pulse
                    }
                    3 => {
                        // LFE: clean deep sub-bass fundamental
                        0.42 * sub * env
                    }
                    4 => {
                        // SL / BL: spacious rear ambient fifth + seventh
                        let swell = 0.5 + 0.5 * (2.0 * PI * 0.18 * t).sin();
                        (0.24 * fifth + 0.20 * seventh) * env * swell
                    }
                    5 => {
                        // SR / BR: complementary rear ambient third + overtone
                        let swell = 0.5 + 0.5 * (2.0 * PI * 0.18 * t + 1.4).sin();
                        (0.24 * third + 0.18 * overtone * 3.0) * env * swell
                    }
                    6 => {
                        // 7.1 Side Left
                        let sw = 0.5 + 0.5 * (2.0 * PI * 0.25 * t + 0.7).sin();
                        (0.20 * seventh + 0.18 * root) * env * sw
                    }
                    _ => {
                        // 7.1 Side Right
                        let sw = 0.5 + 0.5 * (2.0 * PI * 0.25 * t + 2.3).sin();
                        (0.20 * fifth + 0.18 * third) * env * sw
                    }
                };
                ch_buf.push((s * 0.42) as f32);
            }
        }

        self.current_frame += frames as u64;
        Ok(Some(planar))
    }

    fn seek(&mut self, seconds: f64) -> Result<()> {
        let clamped = seconds.clamp(0.0, self.props.duration);
        self.current_frame = (clamped * self.props.decoded_sample_rate as f64) as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::{load_local_lyrics, parse_lyrics};

    #[test]
    fn studio_reference_catalog_decodes_multichannel_and_has_synced_lyrics() {
        assert_eq!(DEMO_CATALOG.len(), 5);
        for spec in DEMO_CATALOG {
            let p = demo_path(spec.id);
            let mut dec = DemoSynthDecoder::open(&p, &DsdSettings::default()).unwrap();
            let props = dec.properties().clone();
            assert_eq!(props.channels, spec.channels);
            let chunk = dec.next_chunk().unwrap().unwrap();
            assert_eq!(chunk.len(), spec.channels as usize);
            assert!(!chunk[0].is_empty());

            dec.seek(15.0).unwrap();
            assert!(dec.next_chunk().unwrap().is_some());

            let built_in_lyrics = parse_lyrics(spec.synced_lyrics, "Studio Reference");
            assert!(built_in_lyrics.is_synced());
            assert!(
                load_local_lyrics(&p).is_some(),
                "studio reference should always resolve lyrics"
            );

            let (w, h, rgba) = generate_swiss_cover_rgba(spec.cover_variant);
            assert_eq!(rgba.len(), w * h * 4);
        }
    }
}
