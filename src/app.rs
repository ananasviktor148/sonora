use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::audio::decoder::{
    is_supported_audio_file, probe_track_info, TrackInfo, SUPPORTED_AUDIO_EXTENSIONS,
};
use crate::audio::demo::{
    demo_spec_for_path, demo_track_info, generate_swiss_cover_rgba, DEMO_CATALOG,
};
use crate::audio::dsp::{ChannelMuteSolo, SpectrumAnalyzer, SPECTRUM_BANDS};
use crate::audio::loudness::{
    invalidate_cached_loudness, resolve_auto_volume, try_resolve_auto_volume_fast, AutoVolumeResult,
};
use crate::audio::ringbuf::{MAX_METER_CHANNELS, VISUALIZER_FFT_SIZE};
use crate::audio::{AudioCommand, AudioEngine, AutoVolumeStatus, PlaybackState, PlaybackStatus};
use crate::config::{
    format_rate, AppConfig, AutoVolumeMode, DownmixMode, DsdOutputMode, EqSettings, FilterRolloff,
    RepeatMode, ResamplerQuality, UpsampleMode, EQ_LABELS, STANDARD_SAMPLE_RATES,
};
use crate::lyrics::{self, is_lyrics_file, LyricsDocument, SUPPORTED_LYRICS_EXTENSIONS};
use crate::playlists::{
    export_m3u8, is_playlist_file, parse_m3u_file, PlaylistEntry, PlaylistLibrary,
};

// ── Swiss Grid · High-Contrast Dark Editorial · Tactile Minimal Palette ──────

const GROUND: egui::Color32 = egui::Color32::from_rgb(9, 9, 11); // #09090B matte carbon
const SURFACE: egui::Color32 = egui::Color32::from_rgb(14, 14, 17); // #0E0E11 architectural panel
const SURFACE_ALT: egui::Color32 = egui::Color32::from_rgb(20, 20, 24); // #141418 tactile lift
const WELL: egui::Color32 = egui::Color32::from_rgb(6, 6, 8); // #060608 recessed meter well
const RULE: egui::Color32 = egui::Color32::from_rgb(35, 35, 42); // #23232A subtle 1px hairline
const RULE_STRONG: egui::Color32 = egui::Color32::from_rgb(58, 58, 68); // #3A3A44 focused 1px hairline

const INK: egui::Color32 = egui::Color32::from_rgb(245, 245, 242); // #F5F5F2 gallery paper white
const INK_DIM: egui::Color32 = egui::Color32::from_rgb(157, 157, 166); // #9D9DA6 editorial neutral
const INK_MUTE: egui::Color32 = egui::Color32::from_rgb(94, 94, 104); // #5E5E68 index & grid captions

const SIGNAL: egui::Color32 = egui::Color32::from_rgb(232, 93, 42); // #E85D2A Braun International Orange
const BRASS: egui::Color32 = egui::Color32::from_rgb(212, 180, 106); // #D4B46A brushed champagne brass
const MINT: egui::Color32 = egui::Color32::from_rgb(60, 200, 140); // #3CC88C bit-perfect / RMS LED
const ALERT: egui::Color32 = egui::Color32::from_rgb(229, 72, 77); // #E5484D mute / clip LED

const EDITORIAL_FONT: &str = "editorial";

// Consistent vertical & horizontal spacing rhythm across all UI sections
const SPACE_XS: f32 = 4.0;
const SPACE_SM: f32 = 8.0;
const SPACE_MD: f32 = 14.0;
const SPACE_LG: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenterTab {
    NowPlaying,
    UpsamplerDsd,
    MultiChannel,
    Equalizer,
}

#[derive(Debug, Clone)]
enum PlaylistEditorMode {
    Create {
        copy_current: bool,
        seed_track: Option<PlaylistEntry>,
    },
    Rename(String),
}

#[derive(Debug, Clone)]
enum LyricsState {
    Empty,
    Loading,
    Loaded(LyricsDocument),
    NotFound,
}

struct ToastMessage {
    text: String,
    is_error: bool,
    expires_at: Instant,
}

pub struct AudioPlayerApp {
    engine: AudioEngine,
    config: AppConfig,
    library: PlaylistLibrary,
    playing_playlist_id: Option<String>,
    current_index: Option<usize>,
    playlist_editor: Option<PlaylistEditorMode>,
    playlist_editor_focus: bool,
    playlist_name_buf: String,
    search_query: String,
    active_tab: CenterTab,
    show_lyrics_panel: bool,

    // Playback state cache
    status: PlaybackStatus,
    track_info: Option<TrackInfo>,
    album_art_texture: Option<egui::TextureHandle>,
    album_art_path: Option<PathBuf>,
    muted: bool,
    seeking: bool,
    seek_preview: f64,

    // Multi-channel mute/solo & metering
    mute_solo: ChannelMuteSolo,
    spectrum: SpectrumAnalyzer,
    fft_scratch: [f32; VISUALIZER_FFT_SIZE],
    smooth_peaks: [f32; MAX_METER_CHANNELS],
    smooth_rms: [f32; MAX_METER_CHANNELS],
    peak_hold: [f32; MAX_METER_CHANNELS],

    // Lyrics state
    lyrics_state: Arc<Mutex<LyricsState>>,
    lyrics_generation: Arc<Mutex<u64>>,
    lyrics_offset_ms: i64,
    last_scrolled_lyric_line: Option<usize>,
    editing_lyrics: bool,
    lyrics_editor_buf: String,

    // Auto Adjust Playback Volume (AudioGate loudness analysis generation)
    auto_volume_generation: Arc<Mutex<u64>>,

    // Notifications
    toast: Option<ToastMessage>,
}

impl AudioPlayerApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        install_editorial_fonts(&cc.egui_ctx);
        apply_swiss_theme(&cc.egui_ctx);

        let config = AppConfig::load();
        let engine = AudioEngine::new(config.clone());
        let library = PlaylistLibrary::load();
        let initial_pl_id = library.active_id.clone();
        let has_tracks = library
            .active()
            .map(|p| !p.tracks.is_empty())
            .unwrap_or(false);

        let mut app = Self {
            engine,
            config,
            library,
            playing_playlist_id: None,
            current_index: None,
            playlist_editor: None,
            playlist_editor_focus: false,
            playlist_name_buf: String::new(),
            search_query: String::new(),
            active_tab: CenterTab::NowPlaying,
            show_lyrics_panel: true,
            status: PlaybackStatus::default(),
            track_info: None,
            album_art_texture: None,
            album_art_path: None,
            muted: false,
            seeking: false,
            seek_preview: 0.0,
            mute_solo: ChannelMuteSolo::default(),
            spectrum: SpectrumAnalyzer::new(),
            fft_scratch: [0.0; VISUALIZER_FFT_SIZE],
            smooth_peaks: [0.0; MAX_METER_CHANNELS],
            smooth_rms: [0.0; MAX_METER_CHANNELS],
            peak_hold: [0.0; MAX_METER_CHANNELS],
            lyrics_state: Arc::new(Mutex::new(LyricsState::Empty)),
            lyrics_generation: Arc::new(Mutex::new(0)),
            lyrics_offset_ms: 0,
            last_scrolled_lyric_line: None,
            editing_lyrics: false,
            lyrics_editor_buf: String::new(),
            auto_volume_generation: Arc::new(Mutex::new(0)),
            toast: None,
        };

        // Pre-stage Track 01 of the active playlist (paused, ready to inspect or play)
        if has_tracks {
            app.load_from_playlist(&initial_pl_id, 0, false);
        }

        app
    }

    pub fn open_initial_paths(&mut self, paths: Vec<PathBuf>) {
        self.ingest_paths(paths, true);
    }

    fn show_toast(&mut self, msg: impl Into<String>, is_error: bool) {
        self.toast = Some(ToastMessage {
            text: msg.into(),
            is_error,
            expires_at: Instant::now() + Duration::from_secs(4),
        });
    }

    fn push_config(&mut self) {
        self.config.save();
        self.engine
            .send(AudioCommand::UpdateConfig(self.config.clone()));
    }

    fn add_audio_files_dialog(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Add Hi-Res Audio Files or Playlists")
            .add_filter("All Audio & Media Formats", SUPPORTED_AUDIO_EXTENSIONS)
            .add_filter("DSD Audio (DSF / DFF)", &["dsf", "dff"])
            .add_filter(
                "Lossless PCM (FLAC / WAV / ALAC / AIFF / WV / APE)",
                &["flac", "wav", "alac", "m4a", "aiff", "aif", "wv", "ape", "caf"],
            )
            .add_filter("Playlists (M3U / M3U8)", &["m3u", "m3u8"])
            .add_filter("All Files", &["*"])
            .pick_files();

        if let Some(paths) = files {
            self.ingest_paths(paths, false);
        }
    }

    fn add_folder_dialog(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Add Music Folder")
            .pick_folder()
        {
            self.ingest_paths(vec![folder], false);
        }
    }

    fn import_m3u_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import Playlist (.m3u / .m3u8)")
            .add_filter("M3U Playlists", &["m3u", "m3u8"])
            .pick_file()
        {
            self.import_m3u_from_path(&path);
        }
    }

    fn import_m3u_from_path(&mut self, path: &Path) {
        match parse_m3u_file(path) {
            Ok((name, entries)) => {
                let count = entries.len();
                let id = self.library.create_playlist(&name, entries);
                self.library.save();
                self.show_toast(
                    format!("IMPORTED PLAYLIST · {} ({} TRACKS)", name.to_uppercase(), count),
                    false,
                );
                if count > 0 && self.status.state != PlaybackState::Playing {
                    self.load_from_playlist(&id, 0, false);
                }
            }
            Err(e) => self.show_toast(format!("IMPORT FAILED · {}", e), true),
        }
    }

    fn export_active_playlist_dialog(&mut self) {
        let Some(pl) = self.library.active().cloned() else {
            return;
        };
        let exportable = pl.tracks.iter().filter(|t| !t.is_demo).count();
        if exportable == 0 {
            self.show_toast(
                "NO LOCAL TRACKS TO EXPORT · ADD AUDIO FILES FIRST",
                true,
            );
            return;
        }
        let default_name = format!("{}.m3u8", pl.name);
        if let Some(dest) = rfd::FileDialog::new()
            .set_title("Export Playlist (.m3u8)")
            .set_file_name(&default_name)
            .add_filter("M3U8 Playlist", &["m3u8"])
            .save_file()
        {
            match export_m3u8(&pl, &dest) {
                Ok(()) => self.show_toast(
                    format!(
                        "EXPORTED {} TRACK{} · {}",
                        exportable,
                        if exportable == 1 { "" } else { "S" },
                        pl.name.to_uppercase()
                    ),
                    false,
                ),
                Err(e) => self.show_toast(format!("EXPORT FAILED · {}", e), true),
            }
        }
    }

    fn start_create_playlist(&mut self, copy_current: bool, seed_track: Option<PlaylistEntry>) {
        self.playlist_name_buf = format!("Playlist {:02}", self.library.playlists.len() + 1);
        self.playlist_editor = Some(PlaylistEditorMode::Create {
            copy_current,
            seed_track,
        });
        self.playlist_editor_focus = true;
    }

    fn start_rename_active_playlist(&mut self) {
        if let Some(pl) = self.library.active() {
            self.playlist_name_buf = pl.name.clone();
            self.playlist_editor = Some(PlaylistEditorMode::Rename(pl.id.clone()));
            self.playlist_editor_focus = true;
        }
    }

    fn commit_playlist_editor(&mut self) {
        let Some(mode) = self.playlist_editor.take() else {
            return;
        };
        let name = self.playlist_name_buf.trim().to_string();
        match mode {
            PlaylistEditorMode::Create {
                copy_current,
                seed_track,
            } => {
                let tracks = if let Some(track) = seed_track {
                    vec![track]
                } else if copy_current {
                    self.library
                        .active()
                        .map(|p| p.tracks.clone())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let count = tracks.len();
                let new_id = self.library.create_playlist(&name, tracks);
                let final_name = self
                    .library
                    .get(&new_id)
                    .map(|p| p.name.clone())
                    .unwrap_or(name);
                self.library.save();
                self.show_toast(
                    if count > 0 {
                        format!(
                            "CREATED PLAYLIST · {} ({} TRACK{})",
                            final_name.to_uppercase(),
                            count,
                            if count == 1 { "" } else { "S" }
                        )
                    } else {
                        format!("CREATED PLAYLIST · {}", final_name.to_uppercase())
                    },
                    false,
                );
            }
            PlaylistEditorMode::Rename(id) => {
                if self.library.rename_playlist(&id, &name) {
                    self.library.save();
                    self.show_toast(
                        format!("RENAMED PLAYLIST · {}", name.to_uppercase()),
                        false,
                    );
                }
            }
        }
        self.playlist_name_buf.clear();
    }

    fn ingest_paths(&mut self, paths: Vec<PathBuf>, autoplay_if_idle: bool) {
        let mut added_audio = 0usize;
        let mut first_new_index: Option<usize> = None;
        let mut dropped_lyrics: Option<PathBuf> = None;
        let mut dropped_playlists: Vec<PathBuf> = Vec::new();

        let mut stack = paths;
        let mut collected = Vec::new();
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    let mut children: Vec<PathBuf> =
                        rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
                    children.sort();
                    children.reverse();
                    stack.extend(children);
                }
            } else {
                collected.push(p);
            }
        }

        let target_pl_id = self.library.active_id.clone();

        // If user is on the untouched Studio Reference playlist (all demo tracks) and drops real audio,
        // replace the demo tracks in that playlist cleanly with their real music.
        let has_real_audio_incoming = collected.iter().any(|p| is_supported_audio_file(p));
        let replace_demo_in_active = has_real_audio_incoming
            && target_pl_id == "pl-studio-reference"
            && self
                .library
                .active()
                .map(|p| !p.tracks.is_empty() && p.tracks.iter().all(|t| t.is_demo))
                .unwrap_or(false);
        if replace_demo_in_active {
            if let Some(pl) = self.library.active_mut() {
                pl.tracks.clear();
            }
            if self.playing_playlist_id.as_deref() == Some(target_pl_id.as_str()) {
                self.current_index = None;
            }
        }

        for p in collected {
            if is_playlist_file(&p) {
                dropped_playlists.push(p);
                continue;
            }
            if is_lyrics_file(&p) {
                dropped_lyrics = Some(p);
                continue;
            }
            if is_supported_audio_file(&p) {
                let info = probe_track_info(&p);
                if let Some(pl) = self.library.get_mut(&target_pl_id) {
                    if first_new_index.is_none() {
                        first_new_index = Some(pl.tracks.len());
                    }
                    pl.tracks.push(PlaylistEntry::from_info(info));
                    added_audio += 1;
                }
            }
        }

        for m3u in dropped_playlists {
            self.import_m3u_from_path(&m3u);
        }

        if let Some(lyrics_path) = dropped_lyrics {
            self.upload_lyrics_from_path(&lyrics_path);
        }

        if added_audio > 0 {
            self.library.save();
            let pl_name = self
                .library
                .get(&target_pl_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "PLAYLIST".into());
            self.show_toast(
                format!(
                    "ADDED {} TRACK{} TO {}",
                    added_audio,
                    if added_audio == 1 { "" } else { "S" },
                    pl_name.to_uppercase()
                ),
                false,
            );
            if (autoplay_if_idle || self.current_index.is_none())
                && self.status.state != PlaybackState::Playing
            {
                if let Some(idx) = first_new_index {
                    self.load_from_playlist(&target_pl_id, idx, true);
                }
            }
        }
    }

    fn load_from_playlist(&mut self, playlist_id: &str, index: usize, autoplay: bool) {
        let entry = match self
            .library
            .get(playlist_id)
            .and_then(|pl| pl.tracks.get(index))
            .cloned()
        {
            Some(e) => e,
            None => return,
        };

        self.playing_playlist_id = Some(playlist_id.to_string());
        self.current_index = Some(index);
        self.album_art_texture = None;
        self.album_art_path = None;
        self.editing_lyrics = false;
        self.lyrics_offset_ms = 0;
        self.last_scrolled_lyric_line = None;
        self.mute_solo = ChannelMuteSolo::default();
        self.engine
            .send(AudioCommand::SetChannelMuteSolo(self.mute_solo.clone()));

        self.engine.send(AudioCommand::Load {
            path: entry.path.clone(),
            start_at: 0.0,
            autoplay,
        });

        self.trigger_auto_volume(Some(entry.path.clone()), false);
        self.load_lyrics_for_track(&entry.path, &entry.artist, &entry.title, false);
    }

    fn collect_album_peer_paths(&self, track_path: &Path) -> Vec<PathBuf> {
        let pid = self.playback_playlist_id();
        let Some(pl) = self.library.get(&pid) else {
            return vec![track_path.to_path_buf()];
        };

        // Determine album name from playlist entry or current TrackInfo
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

    fn auto_volume_cmd_from_result(res: AutoVolumeResult) -> AudioCommand {
        let active = res.mode != AutoVolumeMode::Off;
        AudioCommand::SetAutoVolume {
            gain_db: if active { res.gain_db } else { 0.0 },
            status: AutoVolumeStatus {
                active,
                analyzing: false,
                mode: res.mode,
                applied_gain_db: if active { res.gain_db } else { 0.0 },
                measured_lufs: if active {
                    Some(res.measured_lufs)
                } else {
                    None
                },
                peak_dbfs: res.peak_dbfs,
                peak_limited: res.peak_limited,
                bypassed_for_dop: false,
                source_label: res.source_label,
            },
        }
    }

    fn trigger_auto_volume(&self, explicit_path: Option<PathBuf>, force_rescan: bool) {
        let gen = {
            let mut g = self.auto_volume_generation.lock().unwrap();
            *g += 1;
            *g
        };

        let av_cfg = self.config.auto_volume.clone();
        if av_cfg.mode == AutoVolumeMode::Off {
            self.engine.send(AudioCommand::SetAutoVolume {
                gain_db: 0.0,
                status: AutoVolumeStatus::default(),
            });
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
            self.engine.send(Self::auto_volume_cmd_from_result(fast_res));
            return;
        }

        // Cache miss / rescan requested: run ITU-R BS.1770-4 loudness scan asynchronously
        self.engine
            .send(AudioCommand::SetAutoVolumeAnalyzing(av_cfg.mode));

        let gen_arc = Arc::clone(&self.auto_volume_generation);
        let cmd_tx = self.engine.command_sender();
        let dsd_cfg = self.config.dsd.clone();

        std::thread::spawn(move || {
            let result =
                resolve_auto_volume(&track_path, &album_peers, &av_cfg, &dsd_cfg, force_rescan);
            if *gen_arc.lock().unwrap() != gen {
                return;
            }
            match result {
                Ok(res) => {
                    let _ = cmd_tx.send(Self::auto_volume_cmd_from_result(res));
                }
                Err(e) => {
                    log::warn!("Auto volume loudness scan failed: {}", e);
                    let _ = cmd_tx.send(AudioCommand::SetAutoVolume {
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
                            source_label: "Measurement Unavailable".to_string(),
                        },
                    });
                }
            }
        });
    }

    fn cycle_auto_volume_mode(&mut self) {
        self.config.auto_volume.mode = match self.config.auto_volume.mode {
            AutoVolumeMode::Off => AutoVolumeMode::Song,
            AutoVolumeMode::Song => AutoVolumeMode::Album,
            AutoVolumeMode::Album => AutoVolumeMode::Off,
        };
        self.push_config();
        self.trigger_auto_volume(None, false);
        self.show_toast(
            format!(
                "AUTO ADJUST VOLUME · {}",
                self.config.auto_volume.mode.label().to_uppercase()
            ),
            false,
        );
    }

    fn playback_playlist_id(&self) -> String {
        if let Some(ref pid) = self.playing_playlist_id {
            if self.library.get(pid).is_some() {
                return pid.clone();
            }
        }
        self.library.active_id.clone()
    }

    fn play_next(&mut self, user_triggered: bool) {
        let pid = self.playback_playlist_id();
        let len = self.library.get(&pid).map(|p| p.tracks.len()).unwrap_or(0);
        if len == 0 {
            return;
        }
        if !user_triggered && self.config.repeat_mode == RepeatMode::One {
            if let Some(idx) = self.current_index {
                self.load_from_playlist(&pid, idx.min(len - 1), true);
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
            self.load_from_playlist(&pid, next, true);
            return;
        }

        match self.current_index {
            Some(idx) if idx + 1 < len => self.load_from_playlist(&pid, idx + 1, true),
            Some(_) => {
                if user_triggered || self.config.repeat_mode == RepeatMode::All {
                    self.load_from_playlist(&pid, 0, true);
                }
            }
            None => self.load_from_playlist(&pid, 0, true),
        }
    }

    fn play_prev(&mut self) {
        let pid = self.playback_playlist_id();
        let len = self.library.get(&pid).map(|p| p.tracks.len()).unwrap_or(0);
        if len == 0 {
            return;
        }
        if self.status.position > 3.0 && self.status.state == PlaybackState::Playing {
            self.engine.send(AudioCommand::Seek(0.0));
            return;
        }
        let prev = match self.current_index {
            Some(idx) if idx > 0 => idx - 1,
            _ => len - 1,
        };
        self.load_from_playlist(&pid, prev, true);
    }

    fn current_track_path(&self) -> Option<PathBuf> {
        let pid = self.playback_playlist_id();
        self.current_index
            .and_then(|idx| self.library.get(&pid).and_then(|p| p.tracks.get(idx)))
            .map(|e| e.path.clone())
            .or_else(|| self.track_info.as_ref().map(|t| t.file_path.clone()))
    }

    // ── Lyrics Upload, Fetch, Save & Embed ───────────────────────────────────

    fn upload_lyrics_dialog(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Upload Music Lyrics (.lrc, .txt, .srt, .vtt)")
            .add_filter("Lyrics & Subtitle Files", SUPPORTED_LYRICS_EXTENSIONS)
            .add_filter("All Files", &["*"])
            .pick_file();

        if let Some(lyrics_path) = files {
            self.upload_lyrics_from_path(&lyrics_path);
        }
    }

    fn upload_lyrics_from_path(&mut self, lyrics_path: &Path) {
        let Some(track_path) = self.current_track_path() else {
            match lyrics::read_text_file_auto_encoding(lyrics_path) {
                Ok(raw) => {
                    let fname = lyrics_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("lyrics");
                    let doc = lyrics::parse_lyrics(&raw, format!("Uploaded · {}", fname));
                    *self.lyrics_state.lock().unwrap() = LyricsState::Loaded(doc);
                    self.editing_lyrics = false;
                    self.show_lyrics_panel = true;
                    self.show_toast(format!("LOADED LYRICS · {}", fname), false);
                }
                Err(e) => self.show_toast(format!("READ ERROR · {}", e), true),
            }
            return;
        };

        match lyrics::upload_lyrics_for_track(&track_path, lyrics_path) {
            Ok(doc) => {
                let kind = if doc.is_synced() {
                    format!("{} SYNCED LINES", doc.synced_lines.len())
                } else {
                    format!("{} LINES", doc.plain_lines.len())
                };
                *self.lyrics_state.lock().unwrap() = LyricsState::Loaded(doc);
                self.editing_lyrics = false;
                self.lyrics_offset_ms = 0;
                self.last_scrolled_lyric_line = None;
                self.show_lyrics_panel = true;
                self.show_toast(format!("LYRICS UPLOADED · {}", kind), false);
            }
            Err(e) => {
                self.show_toast(format!("UPLOAD FAILED · {}", e), true);
            }
        }
    }

    fn load_lyrics_for_track(&self, path: &Path, artist: &str, title: &str, force_online: bool) {
        let gen = {
            let mut g = self.lyrics_generation.lock().unwrap();
            *g += 1;
            *g
        };

        if !force_online {
            if let Some(local_doc) = lyrics::load_local_lyrics(path) {
                *self.lyrics_state.lock().unwrap() = LyricsState::Loaded(local_doc);
                return;
            }
        }

        let state_arc = Arc::clone(&self.lyrics_state);
        let gen_arc = Arc::clone(&self.lyrics_generation);
        let artist_s = artist.to_string();
        let title_s = title.to_string();
        *state_arc.lock().unwrap() = LyricsState::Loading;

        std::thread::spawn(move || {
            let result = lyrics::fetch_online_lyrics(&artist_s, &title_s);
            let current_gen = *gen_arc.lock().unwrap();
            if current_gen != gen {
                return;
            }
            let mut guard = state_arc.lock().unwrap();
            *guard = match result {
                Some(doc) => LyricsState::Loaded(doc),
                None => LyricsState::NotFound,
            };
        });
    }

    fn save_edited_lyrics(&mut self) {
        let trimmed = self.lyrics_editor_buf.trim().to_string();
        if trimmed.is_empty() {
            *self.lyrics_state.lock().unwrap() = LyricsState::NotFound;
            self.editing_lyrics = false;
            return;
        }
        let doc = lyrics::parse_lyrics(&trimmed, "Custom Transcript");
        if let Some(track_path) = self.current_track_path() {
            if let Err(e) = lyrics::save_to_lyrics_store(&track_path, &doc.raw_text) {
                self.show_toast(format!("SAVE FAILED · {}", e), true);
            } else {
                self.show_toast("LYRICS SAVED TO LIBRARY", false);
            }
        }
        *self.lyrics_state.lock().unwrap() = LyricsState::Loaded(doc);
        self.editing_lyrics = false;
        self.last_scrolled_lyric_line = None;
    }

    fn save_sidecar_action(&mut self) {
        let Some(track_path) = self.current_track_path() else {
            self.show_toast("NO ACTIVE TRACK FOR SIDECAR", true);
            return;
        };
        if demo_spec_for_path(&track_path).is_some() {
            self.export_lyrics_dialog();
            return;
        }
        let doc_opt = match &*self.lyrics_state.lock().unwrap() {
            LyricsState::Loaded(d) => Some(d.clone()),
            _ => None,
        };
        if let Some(doc) = doc_opt {
            match lyrics::save_sidecar_lyrics(&track_path, &doc) {
                Ok(dest) => self.show_toast(
                    format!(
                        "SAVED SIDECAR · {}",
                        dest.file_name().and_then(|n| n.to_str()).unwrap_or("file")
                    ),
                    false,
                ),
                Err(e) => self.show_toast(format!("SIDECAR ERROR · {}", e), true),
            }
        }
    }

    fn embed_lyrics_action(&mut self) {
        let Some(track_path) = self.current_track_path() else {
            self.show_toast("NO ACTIVE TRACK TO EMBED", true);
            return;
        };
        if demo_spec_for_path(&track_path).is_some() {
            self.show_toast("EMBED AVAILABLE FOR LOCAL AUDIO FILES", true);
            return;
        }
        let raw_opt = match &*self.lyrics_state.lock().unwrap() {
            LyricsState::Loaded(d) => Some(d.raw_text.clone()),
            _ => None,
        };
        if let Some(raw) = raw_opt {
            match lyrics::embed_lyrics_in_audio_file(&track_path, &raw) {
                Ok(()) => self.show_toast("LYRICS EMBEDDED IN AUDIO TAGS", false),
                Err(e) => self.show_toast(format!("EMBED FAILED · {}", e), true),
            }
        }
    }

    fn export_lyrics_dialog(&mut self) {
        let doc_opt = match &*self.lyrics_state.lock().unwrap() {
            LyricsState::Loaded(d) => Some(d.clone()),
            _ => None,
        };
        let Some(doc) = doc_opt else {
            return;
        };
        let default_name = self
            .track_info
            .as_ref()
            .map(|t| {
                format!(
                    "{}.{}",
                    t.title,
                    if doc.is_synced() { "lrc" } else { "txt" }
                )
            })
            .unwrap_or_else(|| "lyrics.lrc".into());

        if let Some(dest) = rfd::FileDialog::new()
            .set_title("Export Lyrics")
            .set_file_name(&default_name)
            .add_filter("LRC Synced Lyrics", &["lrc"])
            .add_filter("Plain Text", &["txt"])
            .save_file()
        {
            match std::fs::write(&dest, &doc.raw_text) {
                Ok(()) => self.show_toast("LYRICS EXPORTED", false),
                Err(e) => self.show_toast(format!("EXPORT FAILED · {}", e), true),
            }
        }
    }

    // ── Visualizer & Artwork Helpers ─────────────────────────────────────────

    fn update_visualizer(&mut self) {
        if self.status.state == PlaybackState::Playing {
            let tap = self.engine.visualizer();
            tap.read_mono_window(&mut self.fft_scratch);
            let sr = self.status.source_sample_rate.clamp(44_100, 384_000);
            self.spectrum.analyze(&self.fft_scratch, sr);

            let (_active, meters) = tap.read_meters();
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
    }

    fn update_album_art(&mut self, ctx: &egui::Context) {
        let Some(ref info) = self.track_info else {
            return;
        };
        if self.album_art_path.as_ref() == Some(&info.file_path) {
            return;
        }
        self.album_art_path = Some(info.file_path.clone());
        self.album_art_texture = None;

        // 1. Embedded artwork from audio file tags
        if let Some(ref bytes) = info.album_art {
            if let Ok(img) = image::load_from_memory(bytes) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.album_art_texture = Some(ctx.load_texture(
                    "album_art",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
                return;
            }
        }

        // 2. Bespoke procedural Swiss modernist sleeve for studio reference tracks
        if let Some(spec) = demo_spec_for_path(&info.file_path) {
            let (w, h, rgba) = generate_swiss_cover_rgba(spec.cover_variant);
            let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            self.album_art_texture = Some(ctx.load_texture(
                "album_art",
                color_image,
                egui::TextureOptions::LINEAR,
            ));
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let mut toggle_play = false;
        let mut seek_delta: Option<f64> = None;
        let mut vol_delta: Option<f32> = None;
        let mut toggle_mute = false;
        let mut next_tr = false;
        let mut prev_tr = false;

        ctx.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                toggle_play = true;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                seek_delta = Some(5.0);
            }
            if i.key_pressed(egui::Key::ArrowLeft) {
                seek_delta = Some(-5.0);
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                vol_delta = Some(0.05);
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                vol_delta = Some(-0.05);
            }
            if i.key_pressed(egui::Key::M) {
                toggle_mute = true;
            }
            if i.key_pressed(egui::Key::N) {
                next_tr = true;
            }
            if i.key_pressed(egui::Key::P) {
                prev_tr = true;
            }
        });

        if toggle_play {
            self.toggle_play_pause();
        }
        if let Some(delta) = seek_delta {
            let target = (self.status.position + delta).clamp(0.0, self.status.duration);
            self.engine.send(AudioCommand::Seek(target));
        }
        if let Some(dv) = vol_delta {
            self.config.volume = (self.config.volume + dv).clamp(0.0, 1.0);
            self.engine
                .send(AudioCommand::SetVolume(self.config.volume));
            self.config.save();
        }
        if toggle_mute {
            self.muted = !self.muted;
            self.engine.send(AudioCommand::SetMuted(self.muted));
        }
        if next_tr {
            self.play_next(true);
        }
        if prev_tr {
            self.play_prev();
        }
    }

    fn toggle_play_pause(&mut self) {
        match self.status.state {
            PlaybackState::Playing => self.engine.send(AudioCommand::Pause),
            PlaybackState::Paused => self.engine.send(AudioCommand::Play),
            PlaybackState::Stopped | PlaybackState::Finished => {
                let pid = self.playback_playlist_id();
                let len = self.library.get(&pid).map(|p| p.tracks.len()).unwrap_or(0);
                if let Some(idx) = self.current_index {
                    if idx < len {
                        self.load_from_playlist(&pid, idx, true);
                        return;
                    }
                }
                let active_id = self.library.active_id.clone();
                if self
                    .library
                    .get(&active_id)
                    .map(|p| !p.tracks.is_empty())
                    .unwrap_or(false)
                {
                    self.load_from_playlist(&active_id, 0, true);
                }
            }
        }
    }
}

impl eframe::App for AudioPlayerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.status = self.engine.status();
        self.track_info = self.engine.track_info();

        if self.status.state == PlaybackState::Finished {
            self.play_next(false);
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.ingest_paths(dropped, true);
        }

        self.handle_keyboard_shortcuts(ctx);
        self.update_album_art(ctx);
        self.update_visualizer();

        if self.status.state == PlaybackState::Playing
            || self.spectrum.bands.iter().any(|&v| v > 0.005)
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        if let Some(ref t) = self.toast {
            if Instant::now() >= t.expires_at {
                self.toast = None;
            }
        }

        self.render_macos_title_bar(ctx);
        self.render_top_bar(ctx);
        self.render_bottom_transport(ctx);
        self.render_playlist_sidebar(ctx);
        if self.show_lyrics_panel {
            self.render_lyrics_panel(ctx);
        }
        self.render_center_stage(ctx);

        // Minimalist Swiss drop overlay (1px dashed/crisp frame, zero blur/drop shadow)
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering_files {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(
                screen,
                egui::CornerRadius::ZERO,
                egui::Color32::from_rgba_unmultiplied(9, 9, 11, 232),
            );
            painter.rect_stroke(
                screen.shrink(32.0),
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0_f32, SIGNAL),
                egui::StrokeKind::Inside,
            );
            painter.text(
                screen.center() - egui::vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                "DROP AUDIO, PLAYLISTS, OR LYRICS",
                egui::FontId::new(22.0, egui::FontFamily::Name(EDITORIAL_FONT.into())),
                INK,
            );
            painter.text(
                screen.center() + egui::vec2(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                "FLAC · DSF · DFF · WAV · ALAC · OPUS · WV · APE · M3U · M3U8 · LRC · SRT · TXT",
                egui::FontId::monospace(11.0),
                SIGNAL,
            );
        }

        self.handle_frameless_window_chrome(ctx);
    }
}

// ── UI Sections (Swiss Grid · Editorial · Tactile Minimal) ───────────────────

impl AudioPlayerApp {
    fn render_macos_title_bar(&mut self, ctx: &egui::Context) {
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let is_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));

        let bar_fill = if is_focused {
            egui::Color32::from_rgb(20, 20, 24)
        } else {
            egui::Color32::from_rgb(14, 14, 17)
        };
        let top_highlight = if is_focused {
            egui::Color32::from_rgb(42, 42, 50)
        } else {
            egui::Color32::from_rgb(26, 26, 32)
        };

        egui::TopBottomPanel::top("macos_title_bar")
            .exact_height(36.0)
            .frame(
                egui::Frame::new()
                    .fill(bar_fill)
                    .inner_margin(egui::Margin::ZERO)
                    .stroke(egui::Stroke::NONE),
            )
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                let cy = rect.center().y;

                // Subtle macOS dark-mode top specular hairline
                painter.line_segment(
                    [
                        egui::pos2(rect.left(), rect.top() + 0.5),
                        egui::pos2(rect.right(), rect.top() + 0.5),
                    ],
                    egui::Stroke::new(1.0_f32, top_highlight),
                );

                // ── Left: Authentic macOS Traffic Lights (Close / Minimize / Zoom) ──
                let x0 = rect.left() + 20.0;
                let spacing = 20.0;
                let c_close = egui::pos2(x0, cy);
                let c_min = egui::pos2(x0 + spacing, cy);
                let c_zoom = egui::pos2(x0 + spacing * 2.0, cy);

                let cluster_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 10.0, cy - 10.0),
                    egui::pos2(x0 + spacing * 2.0 + 10.0, cy + 10.0),
                );
                let cluster_hovered = ui.rect_contains_pointer(cluster_rect);
                let show_color = is_focused || cluster_hovered;

                // Drag & double-click zone (excludes traffic lights and top resize strip)
                let top_inset = if is_maximized { 0.0 } else { 5.0 };
                let drag_rect = egui::Rect::from_min_max(
                    egui::pos2(cluster_rect.right() + 4.0, rect.top() + top_inset),
                    egui::pos2(
                        rect.right() - if is_maximized { 0.0 } else { 5.0 },
                        rect.bottom(),
                    ),
                );
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with("macos_titlebar_drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag_resp.double_clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                } else if drag_resp.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // Helper interaction rects for the 3 traffic-light buttons
                let close_resp = ui
                    .interact(
                        egui::Rect::from_center_size(c_close, egui::vec2(16.0, 16.0)),
                        ui.id().with("macos_btn_close"),
                        egui::Sense::click(),
                    )
                    .on_hover_text("Close");
                let min_resp = ui
                    .interact(
                        egui::Rect::from_center_size(c_min, egui::vec2(16.0, 16.0)),
                        ui.id().with("macos_btn_minimize"),
                        egui::Sense::click(),
                    )
                    .on_hover_text("Minimize");
                let zoom_resp = ui
                    .interact(
                        egui::Rect::from_center_size(c_zoom, egui::vec2(16.0, 16.0)),
                        ui.id().with("macos_btn_zoom"),
                        egui::Sense::click(),
                    )
                    .on_hover_text(if is_maximized { "Restore" } else { "Zoom" });

                if close_resp.hovered() || min_resp.hovered() || zoom_resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                let unfocused_fill = egui::Color32::from_rgb(56, 56, 62);
                let unfocused_rim = egui::Color32::from_rgb(74, 74, 82);
                let r = 6.0_f32;

                // 1. Close button (#FF5F57)
                let close_fill = if !show_color {
                    unfocused_fill
                } else if close_resp.is_pointer_button_down_on() {
                    egui::Color32::from_rgb(191, 73, 67)
                } else {
                    egui::Color32::from_rgb(255, 95, 87)
                };
                let close_rim = if !show_color {
                    unfocused_rim
                } else {
                    egui::Color32::from_rgb(224, 68, 62)
                };
                painter.circle(c_close, r, close_fill, egui::Stroke::new(0.75_f32, close_rim));
                if cluster_hovered {
                    let g = egui::Stroke::new(1.35_f32, egui::Color32::from_rgb(90, 0, 0));
                    let d = 2.6_f32;
                    painter.line_segment(
                        [c_close + egui::vec2(-d, -d), c_close + egui::vec2(d, d)],
                        g,
                    );
                    painter.line_segment(
                        [c_close + egui::vec2(-d, d), c_close + egui::vec2(d, -d)],
                        g,
                    );
                }

                // 2. Minimize button (#FEBC2E)
                let min_fill = if !show_color {
                    unfocused_fill
                } else if min_resp.is_pointer_button_down_on() {
                    egui::Color32::from_rgb(191, 142, 34)
                } else {
                    egui::Color32::from_rgb(254, 188, 46)
                };
                let min_rim = if !show_color {
                    unfocused_rim
                } else {
                    egui::Color32::from_rgb(222, 161, 35)
                };
                painter.circle(c_min, r, min_fill, egui::Stroke::new(0.75_f32, min_rim));
                if cluster_hovered {
                    let g = egui::Stroke::new(1.45_f32, egui::Color32::from_rgb(107, 69, 0));
                    painter.line_segment(
                        [c_min + egui::vec2(-3.2, 0.0), c_min + egui::vec2(3.2, 0.0)],
                        g,
                    );
                }

                // 3. Zoom / Maximize button (#28C840)
                let zoom_fill = if !show_color {
                    unfocused_fill
                } else if zoom_resp.is_pointer_button_down_on() {
                    egui::Color32::from_rgb(31, 154, 49)
                } else {
                    egui::Color32::from_rgb(40, 200, 64)
                };
                let zoom_rim = if !show_color {
                    unfocused_rim
                } else {
                    egui::Color32::from_rgb(26, 171, 41)
                };
                painter.circle(c_zoom, r, zoom_fill, egui::Stroke::new(0.75_f32, zoom_rim));
                if cluster_hovered {
                    let g_col = egui::Color32::from_rgb(0, 77, 0);
                    if is_maximized {
                        // Inward opposing triangles (restore)
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                c_zoom + egui::vec2(0.5, -0.5),
                                c_zoom + egui::vec2(3.2, -0.5),
                                c_zoom + egui::vec2(0.5, -3.2),
                            ],
                            g_col,
                            egui::Stroke::NONE,
                        ));
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                c_zoom + egui::vec2(-0.5, 0.5),
                                c_zoom + egui::vec2(-3.2, 0.5),
                                c_zoom + egui::vec2(-0.5, 3.2),
                            ],
                            g_col,
                            egui::Stroke::NONE,
                        ));
                    } else {
                        // Outward opposing triangles (macOS Zoom / Full-size)
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                c_zoom + egui::vec2(2.7, -2.7),
                                c_zoom + egui::vec2(-0.4, -2.7),
                                c_zoom + egui::vec2(2.7, 0.4),
                            ],
                            g_col,
                            egui::Stroke::NONE,
                        ));
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                c_zoom + egui::vec2(-2.7, 2.7),
                                c_zoom + egui::vec2(0.4, 2.7),
                                c_zoom + egui::vec2(-2.7, -0.4),
                            ],
                            g_col,
                            egui::Stroke::NONE,
                        ));
                    }
                }

                if close_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if min_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if zoom_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                }

                // ── Center: True Geometrically Centered macOS Unified Window Title ──
                let title_ink = if is_focused {
                    egui::Color32::from_rgb(232, 232, 236)
                } else {
                    INK_MUTE
                };
                let subtitle_ink = if is_focused { INK_DIM } else { INK_MUTE };

                let max_sub_chars = ((rect.width() - 360.0) / 7.2).clamp(14.0, 56.0) as usize;
                let subtitle = if let Some(ref info) = self.track_info {
                    let raw = if info.artist.is_empty() {
                        info.title.clone()
                    } else {
                        format!("{} — {}", info.artist, info.title)
                    };
                    truncate_str(&raw, max_sub_chars)
                } else {
                    "Hi-Res Audio Studio".to_string()
                };

                let mut job = egui::text::LayoutJob::default();
                job.append(
                    "Sonora",
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(12.5),
                        color: title_ink,
                        ..Default::default()
                    },
                );
                job.append(
                    "  —  ",
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(12.0),
                        color: INK_MUTE,
                        ..Default::default()
                    },
                );
                job.append(
                    &subtitle,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(12.0),
                        color: subtitle_ink,
                        ..Default::default()
                    },
                );

                let galley = painter.layout_job(job);
                let title_w = galley.size().x;
                let text_pos = egui::pos2(
                    rect.center().x - title_w * 0.5,
                    cy - galley.size().y * 0.5,
                );
                let title_right = text_pos.x + title_w;
                // Small live playback status LED immediately left of the centered title
                let led_col = match self.status.state {
                    PlaybackState::Playing => SIGNAL,
                    PlaybackState::Paused => BRASS,
                    _ => egui::Color32::from_rgb(66, 66, 76),
                };
                if text_pos.x > cluster_rect.right() + 18.0 {
                    painter.circle_filled(egui::pos2(text_pos.x - 10.0, cy), 3.0, led_col);
                    painter.galley(text_pos, galley, title_ink);
                }

                // ── Right: Subtle Studio Output Signal Pill ──
                if rect.width() >= 760.0 {
                    let (pill_text, pill_col) = if let Some(ref info) = self.track_info {
                        if info.is_dsd {
                            (
                                format!(
                                    "{} · {}CH",
                                    info.codec, info.channels
                                ),
                                BRASS,
                            )
                        } else if self.status.output_sample_rate > 0 {
                            (
                                format!(
                                    "{} · {}-BIT · {}",
                                    info.format,
                                    info.bit_depth,
                                    format_rate(self.status.output_sample_rate)
                                ),
                                if self.status.upsampled {
                                    SIGNAL
                                } else if info.is_hires() {
                                    MINT
                                } else {
                                    INK_DIM
                                },
                            )
                        } else {
                            (info.format.clone(), INK_DIM)
                        }
                    } else {
                        ("READY · BIT-PERFECT".to_string(), INK_MUTE)
                    };

                    let pill_galley = painter.layout_no_wrap(
                        pill_text,
                        egui::FontId::monospace(9.5),
                        if is_focused { pill_col } else { INK_MUTE },
                    );
                    let pill_size = pill_galley.size() + egui::vec2(14.0, 6.0);
                    let pill_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 14.0 - pill_size.x * 0.5, cy),
                        pill_size,
                    );
                    if pill_rect.left() > title_right + 16.0 {
                        painter.rect(
                            pill_rect,
                            egui::CornerRadius::same(4),
                            egui::Color32::from_rgb(13, 13, 16),
                            egui::Stroke::new(1.0_f32, RULE),
                            egui::StrokeKind::Inside,
                        );
                        painter.galley(
                            egui::pos2(
                                pill_rect.center().x - pill_galley.size().x * 0.5,
                                pill_rect.center().y - pill_galley.size().y * 0.5,
                            ),
                            pill_galley,
                            pill_col,
                        );
                    }
                }
            });
    }

    fn handle_frameless_window_chrome(&self, ctx: &egui::Context) {
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if is_maximized {
            return;
        }
        let is_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let screen = ctx.screen_rect();

        // Crisp 1px architectural window perimeter hairline
        let fg = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window_perimeter_border"),
        ));
        fg.rect_stroke(
            screen,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0_f32, if is_focused { RULE_STRONG } else { RULE }),
            egui::StrokeKind::Inside,
        );

        // 8-direction invisible edge & corner resize handles
        if ctx.is_using_pointer() {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };

        const EDGE: f32 = 5.0;
        const CORNER: f32 = 10.0;

        let corner_l = pos.x <= screen.left() + CORNER;
        let corner_r = pos.x >= screen.right() - CORNER;
        let corner_t = pos.y <= screen.top() + CORNER;
        let corner_b = pos.y >= screen.bottom() - CORNER;

        let near_l = pos.x <= screen.left() + EDGE;
        let near_r = pos.x >= screen.right() - EDGE;
        let near_t = pos.y <= screen.top() + EDGE;
        let near_b = pos.y >= screen.bottom() - EDGE;

        let resize_dir = if corner_l && corner_t {
            Some((egui::ResizeDirection::NorthWest, egui::CursorIcon::ResizeNwSe))
        } else if corner_r && corner_t {
            Some((egui::ResizeDirection::NorthEast, egui::CursorIcon::ResizeNeSw))
        } else if corner_l && corner_b {
            Some((egui::ResizeDirection::SouthWest, egui::CursorIcon::ResizeNeSw))
        } else if corner_r && corner_b {
            Some((egui::ResizeDirection::SouthEast, egui::CursorIcon::ResizeNwSe))
        } else if near_l {
            Some((egui::ResizeDirection::West, egui::CursorIcon::ResizeHorizontal))
        } else if near_r {
            Some((egui::ResizeDirection::East, egui::CursorIcon::ResizeHorizontal))
        } else if near_t {
            Some((egui::ResizeDirection::North, egui::CursorIcon::ResizeVertical))
        } else if near_b {
            Some((egui::ResizeDirection::South, egui::CursorIcon::ResizeVertical))
        } else {
            None
        };

        if let Some((dir, cursor)) = resize_dir {
            ctx.set_cursor_icon(cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }
    }

    fn render_top_bar(&mut self, ctx: &egui::Context) {
        let compact = ctx.screen_rect().width() < 1220.0;

        egui::TopBottomPanel::top("top_bar")
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(20, 12))
                    .stroke(egui::Stroke::new(1.0_f32, RULE)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = SPACE_SM;

                ui.horizontal(|ui| {
                    // Swiss Masthead: Brand + catalog index
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_SM;
                        let (led_rect, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(led_rect, egui::CornerRadius::same(2), SIGNAL);
                        ui.label(
                            egui::RichText::new("SONORA")
                                .size(14.5)
                                .strong()
                                .color(INK),
                        );
                        ui.label(
                            egui::RichText::new("№ 01")
                                .size(10.0)
                                .monospace()
                                .color(INK_MUTE),
                        );
                    });

                    v_hairline(ui, 20.0);

                    // Clear Primary / Secondary action hierarchy
                    if primary_button(ui, "+ FILES").clicked() {
                        self.add_audio_files_dialog();
                    }
                    if secondary_button(ui, "+ FOLDER", None).clicked() {
                        self.add_folder_dialog();
                    }
                    if secondary_button(ui, "↑ LYRICS", Some(BRASS)).clicked() {
                        self.upload_lyrics_dialog();
                    }

                    if !compact {
                        v_hairline(ui, 20.0);
                        self.render_nav_tabs(ui);
                    }

                    // Right-aligned Output DAC selector & Lyrics toggle
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if toggle_chip(
                            ui,
                            if self.show_lyrics_panel {
                                "LYRICS ●"
                            } else {
                                "LYRICS ○"
                            },
                            self.show_lyrics_panel,
                            BRASS,
                        )
                        .clicked()
                        {
                            self.show_lyrics_panel = !self.show_lyrics_panel;
                        }

                        if ghost_button(ui, "↻", None)
                            .on_hover_text("Refresh audio output devices")
                            .clicked()
                        {
                            self.engine.send(AudioCommand::RefreshDevices);
                        }

                        let devices = self.engine.devices();
                        let selected_label = self
                            .config
                            .selected_device
                            .as_deref()
                            .unwrap_or("DEFAULT DAC");
                        let combo_w = if compact { 150.0 } else { 196.0 };
                        let short_label =
                            truncate_str(selected_label, if compact { 15 } else { 22 });

                        let mut changed_device = false;
                        egui::ComboBox::from_id_salt("dac_device_picker")
                            .width(combo_w)
                            .selected_text(
                                egui::RichText::new(format!("OUT · {}", short_label))
                                    .size(11.0)
                                    .monospace()
                                    .color(INK_DIM),
                            )
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        self.config.selected_device.is_none(),
                                        "System Default Output",
                                    )
                                    .clicked()
                                {
                                    self.config.selected_device = None;
                                    changed_device = true;
                                }
                                ui.separator();
                                for dev in &devices {
                                    let is_sel = self.config.selected_device.as_deref()
                                        == Some(dev.name.as_str());
                                    let label = format!(
                                        "{} ({}ch · {}–{}){}",
                                        dev.name,
                                        dev.max_channels,
                                        format_rate(dev.min_sample_rate),
                                        format_rate(dev.max_sample_rate),
                                        if dev.is_default { " [Default]" } else { "" }
                                    );
                                    if ui.selectable_label(is_sel, label).clicked() {
                                        self.config.selected_device = Some(dev.name.clone());
                                        changed_device = true;
                                    }
                                }
                            });
                        if changed_device {
                            self.push_config();
                            self.show_toast("OUTPUT DEVICE UPDATED", false);
                        }
                    });
                });

                if compact {
                    ui.add_space(SPACE_SM);
                    h_hairline(ui, RULE);
                    ui.add_space(SPACE_SM);
                    ui.horizontal_wrapped(|ui| {
                        self.render_nav_tabs(ui);
                    });
                }
            });
    }

    fn render_nav_tabs(&mut self, ui: &mut egui::Ui) {
        let tabs = [
            (CenterTab::NowPlaying, "01 · PLAYING"),
            (CenterTab::UpsamplerDsd, "02 · UPSAMPLER & DSD"),
            (CenterTab::MultiChannel, "03 · MULTI-CHANNEL"),
            (CenterTab::Equalizer, "04 · EQUALIZER"),
        ];
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        for (tab, label) in tabs {
            if nav_tab_button(ui, label, self.active_tab == tab).clicked() {
                self.active_tab = tab;
            }
        }
    }

    fn render_playlist_sidebar(&mut self, ctx: &egui::Context) {
        let narrow = ctx.screen_rect().width() < 980.0;
        egui::SidePanel::left("playlist_sidebar")
            .default_width(if narrow { 256.0 } else { 324.0 })
            .min_width(220.0)
            .max_width(440.0)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(16, 16))
                    .stroke(egui::Stroke::new(1.0_f32, RULE)),
            )
            .show(ctx, |ui| {
                // ── Playlist Management Group ──
                let active_id = self.library.active_id.clone();
                let active_name = self
                    .library
                    .active()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Queue".into());
                let active_count = self
                    .library
                    .active()
                    .map(|p| p.tracks.len())
                    .unwrap_or(0);

                let mut switch_to_playlist: Option<String> = None;
                let mut do_duplicate = false;
                let mut do_delete = false;
                let mut do_import_m3u = false;
                let mut do_export_m3u = false;
                let mut do_add_refs = false;

                styled_sub_group(ui)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());

                        // Header: PLAYLISTS + [+ NEW] + [⋯] menu
                        ui.horizontal(|ui| {
                            ui.label(swiss_kicker("PLAYLISTS", BRASS));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = SPACE_XS;

                                    egui::Frame::group(ui.style())
                                        .fill(SURFACE_ALT)
                                        .stroke(egui::Stroke::new(1.0_f32, RULE))
                                        .corner_radius(egui::CornerRadius::same(4))
                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                        .show(ui, |ui| {
                                            ui.menu_button(
                                                egui::RichText::new("⋯")
                                                    .size(11.0)
                                                    .monospace()
                                                    .strong()
                                                    .color(INK_DIM),
                                                |ui| {
                                                    if ui.button("Rename Playlist…").clicked() {
                                                        self.start_rename_active_playlist();
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Duplicate Playlist").clicked() {
                                                        do_duplicate = true;
                                                        ui.close_menu();
                                                    }
                                                    ui.separator();
                                                    if ui
                                                        .button("Import Playlist (.m3u / .m3u8)…")
                                                        .clicked()
                                                    {
                                                        do_import_m3u = true;
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Export Playlist (.m3u8)…").clicked()
                                                    {
                                                        do_export_m3u = true;
                                                        ui.close_menu();
                                                    }
                                                    ui.separator();
                                                    if ui
                                                        .button("Add Studio Reference Tracks")
                                                        .clicked()
                                                    {
                                                        do_add_refs = true;
                                                        ui.close_menu();
                                                    }
                                                    if ui
                                                        .button(
                                                            egui::RichText::new("Delete Playlist")
                                                                .color(ALERT),
                                                        )
                                                        .clicked()
                                                    {
                                                        do_delete = true;
                                                        ui.close_menu();
                                                    }
                                                },
                                            );
                                        });

                                    if primary_button(ui, "+ NEW")
                                        .on_hover_text("Create a new playlist")
                                        .clicked()
                                    {
                                        self.start_create_playlist(false, None);
                                    }
                                },
                            );
                        });

                        ui.add_space(SPACE_SM);

                        // Full-width Playlist Switcher ComboBox
                        let combo_w = ui.available_width();
                        egui::ComboBox::from_id_salt("active_playlist_switcher")
                            .width(combo_w)
                            .selected_text(
                                egui::RichText::new(format!(
                                    "{}  ({:02})",
                                    truncate_str(&active_name, 22),
                                    active_count
                                ))
                                .size(12.0)
                                .strong()
                                .color(INK),
                            )
                            .show_ui(ui, |ui| {
                                for pl in &self.library.playlists {
                                    let is_active = pl.id == active_id;
                                    let is_playing_pl =
                                        self.playing_playlist_id.as_deref() == Some(pl.id.as_str())
                                            && self.status.state == PlaybackState::Playing;
                                    let label = format!(
                                        "{}{} ({:02} · {})",
                                        if is_playing_pl { "▶ " } else { "" },
                                        pl.name,
                                        pl.tracks.len(),
                                        format_time(pl.total_duration())
                                    );
                                    if ui.selectable_label(is_active, label).clicked() {
                                        switch_to_playlist = Some(pl.id.clone());
                                    }
                                }
                            });

                        // Inline Create / Rename Playlist Form
                        if let Some(mut mode) = self.playlist_editor.clone() {
                            let mut commit = false;
                            let mut cancel = false;
                            ui.add_space(SPACE_SM);
                            h_hairline(ui, RULE);
                            ui.add_space(SPACE_SM);

                            let form_title = match &mode {
                                PlaylistEditorMode::Create {
                                    seed_track: Some(t),
                                    ..
                                } => format!(
                                    "NEW PLAYLIST WITH '{}'",
                                    truncate_str(&t.title, 18).to_uppercase()
                                ),
                                PlaylistEditorMode::Create { .. } => "NEW PLAYLIST".to_string(),
                                PlaylistEditorMode::Rename(_) => "RENAME PLAYLIST".to_string(),
                            };
                            ui.label(swiss_kicker(&form_title, SIGNAL));
                            ui.add_space(SPACE_XS);

                            let edit_resp = ui.add(
                                egui::TextEdit::singleline(&mut self.playlist_name_buf)
                                    .hint_text("Playlist name…")
                                    .margin(egui::Margin::symmetric(8, 6))
                                    .desired_width(f32::INFINITY),
                            );
                            if self.playlist_editor_focus {
                                edit_resp.request_focus();
                                self.playlist_editor_focus = false;
                            }
                            if edit_resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                commit = true;
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                cancel = true;
                            }

                            if let PlaylistEditorMode::Create {
                                ref mut copy_current,
                                seed_track: None,
                            } = mode
                            {
                                if active_count > 0 {
                                    ui.add_space(SPACE_XS);
                                    ui.checkbox(
                                        copy_current,
                                        egui::RichText::new("Copy tracks from current playlist")
                                            .size(11.0)
                                            .color(INK_DIM),
                                    );
                                }
                            }

                            ui.add_space(SPACE_SM);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = SPACE_SM;
                                let ok_lbl = match mode {
                                    PlaylistEditorMode::Create { .. } => "CREATE",
                                    PlaylistEditorMode::Rename(_) => "RENAME",
                                };
                                if primary_button(ui, ok_lbl).clicked() {
                                    commit = true;
                                }
                                if ghost_button(ui, "CANCEL", None).clicked() {
                                    cancel = true;
                                }
                            });

                            if cancel {
                                self.playlist_editor = None;
                                self.playlist_name_buf.clear();
                            } else if commit {
                                self.playlist_editor = Some(mode);
                                self.commit_playlist_editor();
                            } else {
                                self.playlist_editor = Some(mode);
                            }
                        }
                    });

                // Apply deferred playlist-level actions
                if let Some(pid) = switch_to_playlist {
                    self.library.active_id = pid;
                    self.library.save();
                }
                if do_duplicate {
                    if let Some(dup_id) = self.library.duplicate_playlist(&active_id) {
                        self.library.save();
                        if let Some(pl) = self.library.get(&dup_id) {
                            self.show_toast(
                                format!("DUPLICATED PLAYLIST · {}", pl.name.to_uppercase()),
                                false,
                            );
                        }
                    }
                }
                if do_import_m3u {
                    self.import_m3u_dialog();
                }
                if do_export_m3u {
                    self.export_active_playlist_dialog();
                }
                if do_add_refs {
                    let demos: Vec<PlaylistEntry> = DEMO_CATALOG
                        .iter()
                        .map(|s| PlaylistEntry::from_info(demo_track_info(s)))
                        .collect();
                    if let Some(pl) = self.library.active_mut() {
                        pl.tracks.extend(demos);
                    }
                    self.library.save();
                    self.show_toast("ADDED STUDIO REFERENCE TRACKS", false);
                }
                if do_delete {
                    let deleted_name = active_name.clone();
                    if self.library.delete_playlist(&active_id) {
                        if self.playing_playlist_id.as_deref() == Some(active_id.as_str()) {
                            self.playing_playlist_id = None;
                            self.current_index = None;
                        }
                        self.library.save();
                        self.show_toast(
                            format!("DELETED PLAYLIST · {}", deleted_name.to_uppercase()),
                            false,
                        );
                    }
                }

                ui.add_space(SPACE_MD);

                // Track list summary & Clear button
                let active_id = self.library.active_id.clone();
                let is_viewing_playing_pl =
                    self.playing_playlist_id.as_deref() == Some(active_id.as_str());
                let active_tracks: Vec<PlaylistEntry> = self
                    .library
                    .active()
                    .map(|p| p.tracks.clone())
                    .unwrap_or_default();
                let active_total_dur: f64 = active_tracks.iter().map(|t| t.duration).sum();

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{:02} TRACK{} · {}",
                            active_tracks.len(),
                            if active_tracks.len() == 1 { "" } else { "S" },
                            format_time(active_total_dur)
                        ))
                        .size(10.0)
                        .monospace()
                        .color(INK_MUTE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !active_tracks.is_empty() && ghost_button(ui, "CLEAR", None).clicked() {
                            if let Some(pl) = self.library.active_mut() {
                                pl.tracks.clear();
                            }
                            if is_viewing_playing_pl {
                                self.current_index = None;
                                self.engine.send(AudioCommand::Stop);
                            }
                            self.library.save();
                        }
                    });
                });

                ui.add_space(SPACE_SM);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Filter tracks, artist, format…")
                        .margin(egui::Margin::symmetric(10, 7))
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(SPACE_MD);
                h_hairline(ui, RULE);
                ui.add_space(SPACE_SM);

                if active_tracks.is_empty() {
                    ui.add_space(SPACE_MD);
                    styled_sub_group(ui).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(swiss_kicker("PLAYLIST EMPTY", INK_DIM));
                            ui.add_space(SPACE_SM);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        "Add music files, drop a folder here, or right-click tracks from another playlist to add them.",
                                    )
                                    .size(12.0)
                                    .color(INK_MUTE),
                                )
                                .wrap(),
                            );
                            ui.add_space(SPACE_MD);
                            if primary_button(ui, "+ ADD FILES").clicked() {
                                self.add_audio_files_dialog();
                            }
                            ui.add_space(SPACE_XS);
                            if secondary_button(ui, "ADD STUDIO REFERENCES", Some(BRASS)).clicked()
                            {
                                let demos: Vec<PlaylistEntry> = DEMO_CATALOG
                                    .iter()
                                    .map(|s| PlaylistEntry::from_info(demo_track_info(s)))
                                    .collect();
                                if let Some(pl) = self.library.active_mut() {
                                    pl.tracks.extend(demos);
                                }
                                self.library.save();
                                self.load_from_playlist(&active_id, 0, false);
                            }
                        });
                    });
                    return;
                }

                let query = self.search_query.trim().to_ascii_lowercase();
                let mut to_play: Option<usize> = None;
                let mut to_remove: Option<usize> = None;
                let mut move_up: Option<usize> = None;
                let mut move_down: Option<usize> = None;
                let mut upload_lyrics_for: Option<usize> = None;
                let mut add_to_playlist: Option<(String, PlaylistEntry)> = None;
                let mut new_playlist_with: Option<PlaylistEntry> = None;

                let all_playlists_meta: Vec<(String, String)> = self
                    .library
                    .playlists
                    .iter()
                    .map(|p| (p.id.clone(), p.name.clone()))
                    .collect();

                let panel_w = ui.available_width();
                let title_chars = ((panel_w - 86.0) / 7.2).clamp(13.0, 38.0) as usize;
                let artist_chars = ((panel_w - 150.0) / 6.6).clamp(8.0, 22.0) as usize;

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = SPACE_XS;
                        for (idx, entry) in active_tracks.iter().enumerate() {
                            if !query.is_empty() {
                                let hay = format!(
                                    "{} {} {} {}",
                                    entry.title, entry.artist, entry.format, entry.codec
                                )
                                .to_ascii_lowercase();
                                if !hay.contains(&query) {
                                    continue;
                                }
                            }

                            let is_cur = is_viewing_playing_pl && self.current_index == Some(idx);
                            let row_frame = egui::Frame::group(ui.style())
                                .fill(if is_cur {
                                    SURFACE_ALT
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(if is_cur {
                                    egui::Stroke::new(1.0_f32, RULE_STRONG)
                                } else {
                                    egui::Stroke::new(1.0_f32, egui::Color32::TRANSPARENT)
                                })
                                .corner_radius(egui::CornerRadius::same(5))
                                .inner_margin(egui::Margin::symmetric(10, 9));

                            let resp = row_frame
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = SPACE_SM;
                                        let num_col = if is_cur
                                            && self.status.state == PlaybackState::Playing
                                        {
                                            SIGNAL
                                        } else if is_cur {
                                            INK
                                        } else {
                                            INK_MUTE
                                        };
                                        let lead_txt = if is_cur
                                            && self.status.state == PlaybackState::Playing
                                        {
                                            "▶".to_string()
                                        } else {
                                            format!("{:02}", idx + 1)
                                        };
                                        ui.add_sized(
                                            egui::vec2(20.0, 16.0),
                                            egui::Label::new(
                                                egui::RichText::new(lead_txt)
                                                    .size(10.5)
                                                    .monospace()
                                                    .strong()
                                                    .color(num_col),
                                            ),
                                        );

                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            let mut title_rt = egui::RichText::new(truncate_str(
                                                &entry.title,
                                                title_chars,
                                            ))
                                            .size(13.0)
                                            .color(if is_cur { INK } else { INK_DIM });
                                            if is_cur {
                                                title_rt = title_rt.strong();
                                            }
                                            ui.label(title_rt);
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = SPACE_SM;
                                                if !entry.artist.is_empty() {
                                                    ui.label(
                                                        egui::RichText::new(truncate_str(
                                                            &entry.artist,
                                                            artist_chars,
                                                        ))
                                                        .size(11.0)
                                                        .color(INK_MUTE),
                                                    );
                                                }
                                                let badge_col = if entry.is_dsd {
                                                    BRASS
                                                } else if entry.is_hires {
                                                    MINT
                                                } else {
                                                    INK_MUTE
                                                };
                                                ui.label(
                                                    egui::RichText::new(entry.spec_code())
                                                        .size(9.5)
                                                        .monospace()
                                                        .color(badge_col),
                                                );
                                            });
                                        });

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if entry.duration > 0.0 {
                                                    ui.label(
                                                        egui::RichText::new(format_time(
                                                            entry.duration,
                                                        ))
                                                        .size(10.5)
                                                        .monospace()
                                                        .color(INK_MUTE),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());

                            if is_cur {
                                let r = resp.rect;
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(r.left() + 1.0, r.top() + 6.0),
                                        egui::pos2(r.left() + 3.0, r.bottom() - 6.0),
                                    ),
                                    egui::CornerRadius::same(1),
                                    SIGNAL,
                                );
                            }

                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.double_clicked() {
                                to_play = Some(idx);
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Play Now").clicked() {
                                    to_play = Some(idx);
                                    ui.close_menu();
                                }
                                ui.menu_button("Add to Playlist", |ui| {
                                    for (pid, pname) in &all_playlists_meta {
                                        if ui.button(pname).clicked() {
                                            add_to_playlist = Some((pid.clone(), entry.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                    ui.separator();
                                    if ui.button("+ New Playlist with Track…").clicked() {
                                        new_playlist_with = Some(entry.clone());
                                        ui.close_menu();
                                    }
                                });
                                if ui.button("Upload Lyrics for This Track…").clicked() {
                                    upload_lyrics_for = Some(idx);
                                    ui.close_menu();
                                }
                                ui.separator();
                                if idx > 0 && ui.button("Move Up").clicked() {
                                    move_up = Some(idx);
                                    ui.close_menu();
                                }
                                if idx + 1 < active_tracks.len()
                                    && ui.button("Move Down").clicked()
                                {
                                    move_down = Some(idx);
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("Remove from Playlist").clicked() {
                                    to_remove = Some(idx);
                                    ui.close_menu();
                                }
                            });
                        }
                    });

                if let Some(idx) = to_play {
                    self.load_from_playlist(&active_id, idx, true);
                }
                if let Some((target_pid, entry)) = add_to_playlist {
                    if self.library.add_track_to(&target_pid, entry.clone()) {
                        self.library.save();
                        let pname = self
                            .library
                            .get(&target_pid)
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| "PLAYLIST".into());
                        self.show_toast(
                            format!("ADDED '{}' TO {}", entry.title, pname.to_uppercase()),
                            false,
                        );
                    }
                }
                if let Some(seed) = new_playlist_with {
                    self.start_create_playlist(false, Some(seed));
                }
                if let Some(idx) = upload_lyrics_for {
                    if let Some(entry) = active_tracks.get(idx).cloned() {
                        if let Some(lyrics_path) = rfd::FileDialog::new()
                            .set_title(format!("Upload Lyrics for '{}'", entry.title))
                            .add_filter("Lyrics Files", SUPPORTED_LYRICS_EXTENSIONS)
                            .pick_file()
                        {
                            match lyrics::upload_lyrics_for_track(&entry.path, &lyrics_path) {
                                Ok(doc) => {
                                    if is_viewing_playing_pl && self.current_index == Some(idx) {
                                        *self.lyrics_state.lock().unwrap() =
                                            LyricsState::Loaded(doc);
                                        self.show_lyrics_panel = true;
                                    }
                                    self.show_toast(
                                        format!("UPLOADED LYRICS · {}", entry.title),
                                        false,
                                    );
                                }
                                Err(e) => self.show_toast(format!("UPLOAD FAILED · {}", e), true),
                            }
                        }
                    }
                }
                if let Some(idx) = move_up {
                    if let Some(pl) = self.library.active_mut() {
                        if idx > 0 && idx < pl.tracks.len() {
                            pl.tracks.swap(idx, idx - 1);
                            if is_viewing_playing_pl {
                                if self.current_index == Some(idx) {
                                    self.current_index = Some(idx - 1);
                                } else if self.current_index == Some(idx - 1) {
                                    self.current_index = Some(idx);
                                }
                            }
                            self.library.save();
                        }
                    }
                }
                if let Some(idx) = move_down {
                    if let Some(pl) = self.library.active_mut() {
                        if idx + 1 < pl.tracks.len() {
                            pl.tracks.swap(idx, idx + 1);
                            if is_viewing_playing_pl {
                                if self.current_index == Some(idx) {
                                    self.current_index = Some(idx + 1);
                                } else if self.current_index == Some(idx + 1) {
                                    self.current_index = Some(idx);
                                }
                            }
                            self.library.save();
                        }
                    }
                }
                if let Some(idx) = to_remove {
                    if let Some(pl) = self.library.active_mut() {
                        if idx < pl.tracks.len() {
                            pl.tracks.remove(idx);
                            if is_viewing_playing_pl {
                                if let Some(cur) = self.current_index {
                                    if idx == cur {
                                        self.current_index = None;
                                    } else if idx < cur {
                                        self.current_index = Some(cur - 1);
                                    }
                                }
                            }
                            self.library.save();
                        }
                    }
                }
            });
    }

    fn render_lyrics_panel(&mut self, ctx: &egui::Context) {
        let narrow = ctx.screen_rect().width() < 980.0;
        egui::SidePanel::right("lyrics_studio_panel")
            .default_width(if narrow { 280.0 } else { 380.0 })
            .min_width(240.0)
            .max_width(540.0)
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(18, 16))
                    .stroke(egui::Stroke::new(1.0_f32, RULE)),
            )
            .show(ctx, |ui| {
                // Header
                ui.horizontal(|ui| {
                    ui.label(swiss_kicker("LYRICS / TRANSCRIPT", BRASS));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_XS;

                        egui::Frame::group(ui.style())
                            .fill(SURFACE_ALT)
                            .stroke(egui::Stroke::new(1.0_f32, RULE))
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.menu_button(
                                    egui::RichText::new("SAVE ▾")
                                        .size(10.0)
                                        .monospace()
                                        .strong()
                                        .color(INK_DIM),
                                    |ui| {
                                        if ui.button("Save Sidecar (.lrc / .txt)").clicked() {
                                            self.save_sidecar_action();
                                            ui.close_menu();
                                        }
                                        if ui.button("Embed into Audio File Tags").clicked() {
                                            self.embed_lyrics_action();
                                            ui.close_menu();
                                        }
                                        if ui.button("Export Lyrics File…").clicked() {
                                            self.export_lyrics_dialog();
                                            ui.close_menu();
                                        }
                                    },
                                );
                            });

                        if ghost_button(ui, "WEB", None)
                            .on_hover_text("Search synced lyrics online (LRCLIB)")
                            .clicked()
                        {
                            if let (Some(info), Some(path)) =
                                (self.track_info.clone(), self.current_track_path())
                            {
                                self.load_lyrics_for_track(&path, &info.artist, &info.title, true);
                            } else {
                                self.show_toast("LOAD A TRACK FIRST TO FETCH LYRICS", true);
                            }
                        }

                        if secondary_button(
                            ui,
                            if self.editing_lyrics { "DONE" } else { "EDIT" },
                            if self.editing_lyrics {
                                Some(SIGNAL)
                            } else {
                                None
                            },
                        )
                        .clicked()
                        {
                            self.editing_lyrics = !self.editing_lyrics;
                            if self.editing_lyrics {
                                let guard = self.lyrics_state.lock().unwrap();
                                self.lyrics_editor_buf = match &*guard {
                                    LyricsState::Loaded(doc) => doc.raw_text.clone(),
                                    _ => String::new(),
                                };
                            }
                        }

                        if primary_button(ui, "↑ UPLOAD")
                            .on_hover_text("Upload .lrc, .txt, .srt, or .vtt lyrics file")
                            .clicked()
                        {
                            self.upload_lyrics_dialog();
                        }
                    });
                });

                ui.add_space(SPACE_MD);

                if self.editing_lyrics {
                    self.render_lyrics_editor(ui);
                    return;
                }

                let state_snapshot = self.lyrics_state.lock().unwrap().clone();
                match state_snapshot {
                    LyricsState::Empty | LyricsState::NotFound => {
                        ui.add_space(SPACE_MD);
                        styled_sub_group(ui).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(swiss_kicker("TRANSCRIPT STATUS", INK_MUTE));
                            ui.add_space(SPACE_SM);
                            let msg = if matches!(state_snapshot, LyricsState::NotFound) {
                                "No transcript attached to this track."
                            } else {
                                "Attach a lyrics file or write a transcript."
                            };
                            ui.add(egui::Label::new(editorial_text(msg, 18.0, INK)).wrap());
                            ui.add_space(SPACE_SM);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        "Supports time-synced .lrc, .srt, .vtt, and plain .txt files. Drop a file anywhere onto the window.",
                                    )
                                    .size(12.0)
                                    .color(INK_MUTE),
                                )
                                .wrap(),
                            );
                            ui.add_space(SPACE_MD);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                                if primary_button(ui, "↑ UPLOAD LYRICS").clicked() {
                                    self.upload_lyrics_dialog();
                                }
                                if secondary_button(ui, "WRITE / PASTE", None).clicked() {
                                    self.lyrics_editor_buf.clear();
                                    self.editing_lyrics = true;
                                }
                            });
                        });
                    }
                    LyricsState::Loading => {
                        ui.add_space(SPACE_LG * 2.0);
                        ui.vertical_centered(|ui| {
                            ui.spinner();
                            ui.add_space(SPACE_MD);
                            ui.label(swiss_kicker("QUERYING LRCLIB…", INK_DIM));
                        });
                    }
                    LyricsState::Loaded(doc) => {
                        // Grouped, grid-aligned transcript metadata & sync controls
                        styled_sub_group(ui)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::Grid::new("lyrics_controls_grid")
                                    .num_columns(2)
                                    .spacing([SPACE_MD, SPACE_SM])
                                    .show(ui, |ui| {
                                        ui.label(swiss_kicker("SOURCE", INK_MUTE));
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = SPACE_XS;
                                            let badge = if doc.is_synced() {
                                                format!("SYNCED · {}", doc.source.to_uppercase())
                                            } else {
                                                format!("PLAIN · {}", doc.source.to_uppercase())
                                            };
                                            let max_chars = ((ui.available_width() - 68.0) / 6.3)
                                                .clamp(12.0, 28.0)
                                                as usize;
                                            ui.label(
                                                egui::RichText::new(truncate_str(
                                                    &badge, max_chars,
                                                ))
                                                .size(9.5)
                                                .monospace()
                                                .strong()
                                                .color(if doc.is_synced() {
                                                    MINT
                                                } else {
                                                    INK_MUTE
                                                }),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                                                    if ghost_button(ui, "A+", None).clicked() {
                                                        self.config.lyrics_font_size =
                                                            (self.config.lyrics_font_size + 1.5)
                                                                .min(28.0);
                                                        self.config.save();
                                                    }
                                                    if ghost_button(ui, "A-", None).clicked() {
                                                        self.config.lyrics_font_size =
                                                            (self.config.lyrics_font_size - 1.5)
                                                                .max(12.0);
                                                        self.config.save();
                                                    }
                                                },
                                            );
                                        });
                                        ui.end_row();

                                        if doc.is_synced() {
                                            ui.label(swiss_kicker("SYNC", INK_MUTE));
                                            ui.horizontal_wrapped(|ui| {
                                                ui.spacing_mut().item_spacing =
                                                    egui::vec2(SPACE_XS, SPACE_XS);
                                                if ghost_button(ui, "-.25", None).clicked() {
                                                    self.lyrics_offset_ms -= 250;
                                                }
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{:+.2}s",
                                                        self.lyrics_offset_ms as f64 / 1000.0
                                                    ))
                                                    .size(10.5)
                                                    .monospace()
                                                    .color(INK),
                                                );
                                                if ghost_button(ui, "+.25", None).clicked() {
                                                    self.lyrics_offset_ms += 250;
                                                }
                                                if self.lyrics_offset_ms != 0
                                                    && ghost_button(ui, "0", None).clicked()
                                                {
                                                    self.lyrics_offset_ms = 0;
                                                }
                                                ui.add_space(SPACE_XS);
                                                if ui
                                                    .checkbox(
                                                        &mut self.config.lyrics_auto_scroll,
                                                        egui::RichText::new("FOLLOW")
                                                            .size(9.5)
                                                            .monospace()
                                                            .color(INK_DIM),
                                                    )
                                                    .changed()
                                                {
                                                    self.config.save();
                                                }
                                            });
                                            ui.end_row();
                                        }
                                    });
                            });

                        ui.add_space(SPACE_MD);

                        let base_font = self.config.lyrics_font_size;
                        let active_idx =
                            doc.active_line_index(self.status.position, self.lyrics_offset_ms);
                        let should_scroll = self.config.lyrics_auto_scroll
                            && active_idx.is_some()
                            && active_idx != self.last_scrolled_lyric_line;
                        if should_scroll {
                            self.last_scrolled_lyric_line = active_idx;
                        }

                        let mut seek_to: Option<f64> = None;

                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = SPACE_XS;
                                ui.add_space(SPACE_XS);
                                if doc.is_synced() {
                                    for (idx, line) in doc.synced_lines.iter().enumerate() {
                                        let is_active = active_idx == Some(idx);
                                        let is_past = active_idx.map(|a| idx < a).unwrap_or(false);

                                        let text_display = if line.text.is_empty() {
                                            "—"
                                        } else {
                                            &line.text
                                        };

                                        let frame = egui::Frame::group(ui.style())
                                            .fill(if is_active {
                                                SURFACE_ALT
                                            } else {
                                                egui::Color32::TRANSPARENT
                                            })
                                            .stroke(if is_active {
                                                egui::Stroke::new(1.0_f32, RULE_STRONG)
                                            } else {
                                                egui::Stroke::NONE
                                            })
                                            .corner_radius(egui::CornerRadius::same(5))
                                            .inner_margin(egui::Margin::symmetric(12, 8));

                                        let resp = frame
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.horizontal_top(|ui| {
                                                    ui.spacing_mut().item_spacing.x = SPACE_MD;
                                                    ui.add_sized(
                                                        egui::vec2(38.0, 16.0),
                                                        egui::Label::new(
                                                            egui::RichText::new(format_time(
                                                                line.time_secs,
                                                            ))
                                                            .size(10.0)
                                                            .monospace()
                                                            .color(if is_active {
                                                                SIGNAL
                                                            } else {
                                                                INK_MUTE
                                                            }),
                                                        ),
                                                    );
                                                    let rt = if is_active {
                                                        editorial_text(
                                                            text_display,
                                                            base_font + 3.5,
                                                            BRASS,
                                                        )
                                                    } else if is_past {
                                                        editorial_text(
                                                            text_display,
                                                            base_font - 0.5,
                                                            INK_MUTE,
                                                        )
                                                    } else {
                                                        editorial_text(
                                                            text_display,
                                                            base_font,
                                                            INK,
                                                        )
                                                    };
                                                    ui.add(egui::Label::new(rt).wrap());
                                                });
                                            })
                                            .response
                                            .interact(egui::Sense::click())
                                            .on_hover_text(format!(
                                                "Seek to {}",
                                                format_time(line.time_secs)
                                            ));

                                        if is_active {
                                            let r = resp.rect;
                                            ui.painter().rect_filled(
                                                egui::Rect::from_min_max(
                                                    egui::pos2(r.left() + 1.0, r.top() + 5.0),
                                                    egui::pos2(r.left() + 3.0, r.bottom() - 5.0),
                                                ),
                                                egui::CornerRadius::same(1),
                                                SIGNAL,
                                            );
                                        }
                                        if resp.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if resp.clicked() {
                                            seek_to = Some(line.time_secs);
                                        }
                                        if is_active && should_scroll {
                                            resp.scroll_to_me(Some(egui::Align::Center));
                                        }
                                    }
                                } else {
                                    for line in &doc.plain_lines {
                                        if line.trim().is_empty() {
                                            ui.add_space(SPACE_MD);
                                        } else {
                                            ui.add(
                                                egui::Label::new(editorial_text(
                                                    line, base_font, INK,
                                                ))
                                                .wrap(),
                                            );
                                            ui.add_space(SPACE_XS);
                                        }
                                    }
                                }
                                ui.add_space(SPACE_LG * 2.0);
                            });

                        if let Some(t) = seek_to {
                            self.engine.send(AudioCommand::Seek(t));
                        }
                    }
                }
            });
    }

    fn render_lyrics_editor(&mut self, ui: &mut egui::Ui) {
        styled_sub_group(ui)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Edit plain or [mm:ss.xx] LRC lyrics. Use STAMP while playing to sync lines.",
                        )
                        .size(11.5)
                        .color(INK_DIM),
                    )
                    .wrap(),
                );
                ui.add_space(SPACE_SM);

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                    let stamp = lyrics::format_lrc_timestamp(self.status.position);
                    if secondary_button(ui, &format!("⏱ STAMP {}", stamp), Some(SIGNAL)).clicked()
                    {
                        if !self.lyrics_editor_buf.is_empty()
                            && !self.lyrics_editor_buf.ends_with('\n')
                        {
                            self.lyrics_editor_buf.push('\n');
                        }
                        self.lyrics_editor_buf.push_str(&stamp);
                        self.lyrics_editor_buf.push(' ');
                    }
                    if secondary_button(ui, "↑ LOAD FILE", Some(BRASS)).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("Lyrics Files", SUPPORTED_LYRICS_EXTENSIONS)
                            .pick_file()
                        {
                            if let Ok(raw) = lyrics::read_text_file_auto_encoding(&p) {
                                self.lyrics_editor_buf = raw;
                            }
                        }
                    }
                });
            });

        ui.add_space(SPACE_MD);
        let avail_h = (ui.available_height() - 52.0).max(120.0);
        egui::ScrollArea::vertical()
            .max_height(avail_h)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.lyrics_editor_buf)
                        .desired_width(f32::INFINITY)
                        .desired_rows(18)
                        .margin(egui::Margin::symmetric(12, 10))
                        .font(egui::TextStyle::Monospace),
                );
            });

        ui.add_space(SPACE_MD);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_SM;
            if primary_button(ui, "SAVE TRANSCRIPT").clicked() {
                self.save_edited_lyrics();
            }
            if ghost_button(ui, "CANCEL", None).clicked() {
                self.editing_lyrics = false;
            }
        });
    }

    fn render_center_stage(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(GROUND)
                    .inner_margin(egui::Margin::symmetric(28, 22)),
            )
            .show(ctx, |ui| {
                // Status / Toast grouped banner
                if let Some(ref err) = self.status.error_message {
                    egui::Frame::group(ui.style())
                        .fill(SURFACE_ALT)
                        .stroke(egui::Stroke::new(1.0_f32, ALERT))
                        .corner_radius(egui::CornerRadius::same(5))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(err)
                                        .size(11.5)
                                        .monospace()
                                        .color(ALERT),
                                )
                                .wrap(),
                            );
                        });
                    ui.add_space(SPACE_MD);
                }
                if let Some(ref toast) = self.toast {
                    let accent = if toast.is_error { ALERT } else { SIGNAL };
                    egui::Frame::group(ui.style())
                        .fill(SURFACE)
                        .stroke(egui::Stroke::new(1.0_f32, accent))
                        .corner_radius(egui::CornerRadius::same(5))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = SPACE_SM;
                                ui.label(egui::RichText::new("■").size(9.0).color(accent));
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&toast.text)
                                            .size(11.0)
                                            .monospace()
                                            .color(INK),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    ui.add_space(SPACE_MD);
                }

                match self.active_tab {
                    CenterTab::NowPlaying => self.render_now_playing_tab(ui),
                    CenterTab::UpsamplerDsd => self.render_upsampler_dsd_tab(ui),
                    CenterTab::MultiChannel => self.render_multichannel_tab(ui),
                    CenterTab::Equalizer => self.render_eq_tab(ui),
                }
            });
    }

    fn render_artwork_plate(&self, ui: &mut egui::Ui, info: &TrackInfo, art_size: f32) {
        let plate_num = self.current_index.map(|i| i + 1).unwrap_or(1);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = SPACE_SM;
            egui::Frame::group(ui.style())
                .fill(WELL)
                .corner_radius(egui::CornerRadius::same(6))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    if info.is_dsd { BRASS } else { RULE_STRONG },
                ))
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    if let Some(ref tex) = self.album_art_texture {
                        ui.add(
                            egui::Image::new(tex)
                                .fit_to_exact_size(egui::vec2(art_size, art_size))
                                .corner_radius(egui::CornerRadius::same(4)),
                        );
                    } else {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(art_size, art_size),
                            egui::Sense::hover(),
                        );
                        let p = ui.painter();
                        p.rect_filled(rect, egui::CornerRadius::same(4), SURFACE);
                        p.circle_stroke(
                            rect.center(),
                            art_size * 0.34,
                            egui::Stroke::new(1.0_f32, RULE_STRONG),
                        );
                        p.circle_filled(rect.center(), art_size * 0.08, SIGNAL);
                    }
                });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                ui.label(
                    egui::RichText::new(format!("PLATE {:02}", plate_num))
                        .size(9.5)
                        .monospace()
                        .color(INK_MUTE),
                );
                if demo_spec_for_path(&info.file_path).is_some() {
                    ui.label(
                        egui::RichText::new("· STUDIO REF")
                            .size(9.5)
                            .monospace()
                            .color(BRASS),
                    );
                }
            });
        });
    }

    fn render_track_hero_details(&mut self, ui: &mut egui::Ui, info: &TrackInfo) {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());

            // Minimalist certification tags
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_XS);
                if info.is_dsd {
                    stamp_tag(ui, &info.codec, BRASS, BRASS);
                } else if info.is_hires() {
                    stamp_tag(ui, "HI-RES MASTER", BRASS, BRASS);
                }
                if self.status.is_dop {
                    stamp_tag(ui, "DoP v1.1", MINT, MINT);
                }
                if self.status.upsampled {
                    stamp_tag(
                        ui,
                        &format!(
                            "↑ {}",
                            format_rate(self.status.output_sample_rate).to_uppercase()
                        ),
                        SIGNAL,
                        SIGNAL,
                    );
                }
                if info.channels > 2 {
                    stamp_tag(
                        ui,
                        &info.channel_layout_name().to_uppercase(),
                        INK,
                        RULE_STRONG,
                    );
                }
                if self.status.downmixed {
                    stamp_tag(
                        ui,
                        &format!(
                            "{}→{}CH MATRIX",
                            self.status.source_channels, self.status.output_channels
                        ),
                        INK_DIM,
                        RULE,
                    );
                }
                if self.status.signal_path.bit_perfect {
                    stamp_tag(ui, "● BIT-PERFECT", MINT, MINT);
                }
            });

            ui.add_space(SPACE_MD);

            // Editorial Serif Display Title + Artist + Album
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = SPACE_XS;
                ui.add(egui::Label::new(editorial_text(&info.title, 30.0, INK)).wrap());
                if !info.artist.is_empty() {
                    ui.add(egui::Label::new(editorial_text(&info.artist, 17.5, INK_DIM)).wrap());
                }
                if !info.album.is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(info.album.to_uppercase())
                                .size(10.5)
                                .monospace()
                                .color(INK_MUTE),
                        )
                        .wrap(),
                    );
                }
            });

            ui.add_space(SPACE_MD);

            // Specification Grid inside styled egui::Frame::group(ui.style())
            let bit_txt = if info.is_dsd {
                "1-BIT DSD".to_string()
            } else {
                format!("{}-BIT PCM", info.bit_depth)
            };
            let rate_txt = if self.status.output_sample_rate > 0
                && self.status.output_sample_rate != info.sample_rate
            {
                format!(
                    "{} → {}",
                    format_rate(info.sample_rate),
                    format_rate(self.status.output_sample_rate)
                )
            } else {
                format_rate(info.sample_rate)
            };
            let ch_txt = if self.status.output_channels > 0
                && self.status.output_channels != info.channels
            {
                format!("{} → {}CH", info.channels, self.status.output_channels)
            } else {
                info.channel_layout_name().to_uppercase()
            };

            let spec_cells = [
                ("FORMAT", format!("{} · {}", info.format, info.codec), INK),
                (
                    "SAMPLE RATE",
                    rate_txt,
                    if self.status.upsampled { SIGNAL } else { INK },
                ),
                (
                    "DEPTH",
                    bit_txt,
                    if info.is_dsd { BRASS } else { INK },
                ),
                ("CHANNELS", ch_txt, INK),
            ];

            let cols = if ui.available_width() < 430.0 { 2 } else { 4 };
            let cell_w = ((ui.available_width() - 28.0 - (cols as f32 - 1.0) * SPACE_MD)
                / cols as f32)
                .max(92.0);

            styled_sub_group(ui)
                .fill(SURFACE)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::Grid::new("hero_spec_grid")
                        .num_columns(cols)
                        .spacing([SPACE_MD, SPACE_SM])
                        .min_col_width(cell_w)
                        .show(ui, |ui| {
                            for (i, (k, v, col)) in spec_cells.iter().enumerate() {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = SPACE_XS;
                                    ui.label(
                                        egui::RichText::new(*k)
                                            .size(9.0)
                                            .monospace()
                                            .color(INK_MUTE),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(v)
                                                .size(11.5)
                                                .monospace()
                                                .strong()
                                                .color(*col),
                                        )
                                        .wrap(),
                                    );
                                });
                                if (i + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });

            ui.add_space(SPACE_MD);

            // Secondary DSP quick-jump actions
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                let up_lbl = format!(
                    "UPSAMPLER · {}",
                    self.config.upsample_mode.label().to_uppercase()
                );
                if secondary_button(
                    ui,
                    &up_lbl,
                    if self.config.upsample_mode != UpsampleMode::Bypass {
                        Some(SIGNAL)
                    } else {
                        None
                    },
                )
                .clicked()
                {
                    self.active_tab = CenterTab::UpsamplerDsd;
                }

                let eq_lbl = if self.config.eq.enabled {
                    format!("EQ · {}", self.config.eq.preset_name.to_uppercase())
                } else {
                    "EQ · BYPASS".into()
                };
                if secondary_button(
                    ui,
                    &eq_lbl,
                    if self.config.eq.enabled {
                        Some(BRASS)
                    } else {
                        None
                    },
                )
                .clicked()
                {
                    self.active_tab = CenterTab::Equalizer;
                }

                let av_mode = self.config.auto_volume.mode;
                let av_lbl = match av_mode {
                    AutoVolumeMode::Off => "AUTO VOL · OFF".to_string(),
                    AutoVolumeMode::Song | AutoVolumeMode::Album => {
                        if self.status.auto_volume.bypassed_for_dop {
                            format!("AUTO VOL · {} (DoP)", av_mode.short_label())
                        } else if self.status.auto_volume.analyzing {
                            format!("AUTO VOL · {} (SCAN)", av_mode.short_label())
                        } else if self.status.auto_volume.active {
                            format!(
                                "AUTO VOL · {} ({:+.1} dB)",
                                av_mode.short_label(),
                                self.status.auto_volume.applied_gain_db
                            )
                        } else {
                            format!("AUTO VOL · {}", av_mode.short_label())
                        }
                    }
                };
                let av_accent = match av_mode {
                    AutoVolumeMode::Off => None,
                    AutoVolumeMode::Song => Some(MINT),
                    AutoVolumeMode::Album => Some(BRASS),
                };
                if secondary_button(ui, &av_lbl, av_accent)
                    .on_hover_text("Auto Adjust Playback Volume (AudioGate Loudness Engine)")
                    .clicked()
                {
                    self.active_tab = CenterTab::UpsamplerDsd;
                }

                if info.channels > 2 {
                    let mc_lbl = format!(
                        "MATRIX · {}",
                        self.config.multichannel.downmix_mode.label().to_uppercase()
                    );
                    if secondary_button(ui, &mc_lbl, None).clicked() {
                        self.active_tab = CenterTab::MultiChannel;
                    }
                }
            });
        });
    }

    fn render_now_playing_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(info) = self.track_info.clone() {
                    let narrow_center = ui.available_width() < 560.0;

                    if narrow_center {
                        ui.vertical_centered(|ui| {
                            self.render_artwork_plate(ui, &info, 190.0);
                        });
                        ui.add_space(SPACE_LG);
                        self.render_track_hero_details(ui, &info);
                    } else {
                        ui.horizontal_top(|ui| {
                            self.render_artwork_plate(ui, &info, 206.0);
                            ui.add_space(SPACE_LG);
                            self.render_track_hero_details(ui, &info);
                        });
                    }

                    ui.add_space(SPACE_LG);

                    // 01 / Real-Time 48-Band FFT Spectrum Analyzer
                    swiss_module(ui, "01", "REAL-TIME SPECTRUM ANALYZER · 2048-PT FFT", |ui| {
                        self.paint_spectrum_analyzer(ui, 152.0);
                    });

                    ui.add_space(SPACE_LG);

                    // 02 / Multi-Channel Peak / RMS Meter Bridge
                    let ch_count = info.channels.clamp(1, MAX_METER_CHANNELS as u16) as usize;
                    let ch_labels = info.channel_labels.clone();
                    swiss_module(
                        ui,
                        "02",
                        &format!(
                            "CHANNEL METER BRIDGE · {}",
                            info.channel_layout_name().to_uppercase()
                        ),
                        |ui| {
                            self.paint_horizontal_channel_meters(ui, ch_count, &ch_labels);
                        },
                    );

                    ui.add_space(SPACE_LG);

                    // 03 / Audio Signal Chain (cleanly aligned with egui::Grid)
                    if !self.status.signal_path.stages.is_empty() {
                        swiss_module(ui, "03", "AUDIO SIGNAL CHAIN", |ui| {
                            egui::Grid::new("signal_path_grid")
                                .num_columns(3)
                                .spacing([SPACE_MD, SPACE_SM])
                                .show(ui, |ui| {
                                    for stage in &self.status.signal_path.stages {
                                        let dot_col = if stage.lossless { MINT } else { BRASS };
                                        ui.label(egui::RichText::new("■").size(8.5).color(dot_col));
                                        ui.label(
                                            egui::RichText::new(stage.title.to_uppercase())
                                                .size(11.0)
                                                .strong()
                                                .monospace()
                                                .color(INK),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&stage.detail)
                                                    .size(11.5)
                                                    .monospace()
                                                    .color(INK_DIM),
                                            )
                                            .wrap(),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                    }
                    ui.add_space(SPACE_MD);
                } else {
                    ui.add_space(SPACE_LG);
                    swiss_module(ui, "00", "SONORA HI-RES MASTERING CONSOLE", |ui| {
                        ui.add(
                            egui::Label::new(editorial_text(
                                "Open local audio files or restore the studio reference catalog.",
                                24.0,
                                INK,
                            ))
                            .wrap(),
                        );
                        ui.add_space(SPACE_SM);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(if self.engine.ffmpeg_available() {
                                    "ENGINE READY · SYMPHONIA NATIVE PCM + NATIVE DSD (DSF/DFF) + FFMPEG UNIVERSAL CODECS"
                                } else {
                                    "ENGINE READY · SYMPHONIA NATIVE PCM + NATIVE DSD (DSF/DFF)"
                                })
                                .size(10.5)
                                .monospace()
                                .color(MINT),
                            )
                            .wrap(),
                        );
                        ui.add_space(SPACE_LG);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                            if primary_button(ui, "+ OPEN MUSIC FILES").clicked() {
                                self.add_audio_files_dialog();
                            }
                            if secondary_button(ui, "+ OPEN FOLDER", None).clicked() {
                                self.add_folder_dialog();
                            }
                            if secondary_button(ui, "RESTORE STUDIO REFERENCES", Some(BRASS))
                                .clicked()
                            {
                                let demos: Vec<PlaylistEntry> = DEMO_CATALOG
                                    .iter()
                                    .map(|s| PlaylistEntry::from_info(demo_track_info(s)))
                                    .collect();
                                let pid = self.library.active_id.clone();
                                if let Some(pl) = self.library.active_mut() {
                                    pl.tracks = demos;
                                }
                                self.library.save();
                                self.load_from_playlist(&pid, 0, false);
                            }
                        });
                    });
                }
            });
    }

    fn render_upsampler_dsd_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let mut changed = false;
                let mut auto_vol_changed = false;
                let mut force_rescan = false;

                swiss_module(
                    ui,
                    "01",
                    "HI-RES SINC UPSAMPLER · 64-BIT FLOATING-POINT INTERNAL",
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Select oversampling ratio or fixed target rate. Changes engage seamlessly at the current playhead.",
                                )
                                .size(12.5)
                                .color(INK_DIM),
                            )
                            .wrap(),
                        );
                        ui.add_space(SPACE_MD);

                        egui::Grid::new("upsampler_controls_grid")
                            .num_columns(2)
                            .spacing([SPACE_LG, SPACE_MD])
                            .show(ui, |ui| {
                                ui.label(swiss_kicker("MODE", INK_MUTE));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                                    let modes = [
                                        (UpsampleMode::Bypass, "BYPASS (BIT-PERFECT)"),
                                        (UpsampleMode::Factor2x, "2× INTEGER"),
                                        (UpsampleMode::Factor4x, "4× INTEGER"),
                                        (UpsampleMode::Factor8x, "8× INTEGER"),
                                    ];
                                    for (m, lbl) in modes {
                                        if toggle_chip(
                                            ui,
                                            lbl,
                                            self.config.upsample_mode == m,
                                            SIGNAL,
                                        )
                                        .clicked()
                                        {
                                            self.config.upsample_mode = m;
                                            changed = true;
                                        }
                                    }
                                    let is_fixed = matches!(
                                        self.config.upsample_mode,
                                        UpsampleMode::Fixed(_)
                                    );
                                    if toggle_chip(ui, "FIXED RATE", is_fixed, BRASS).clicked()
                                        && !is_fixed
                                    {
                                        self.config.upsample_mode = UpsampleMode::Fixed(192_000);
                                        changed = true;
                                    }
                                });
                                ui.end_row();

                                if let UpsampleMode::Fixed(cur_rate) = self.config.upsample_mode {
                                    ui.label(swiss_kicker("TARGET RATE", INK_MUTE));
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing =
                                            egui::vec2(SPACE_SM, SPACE_SM);
                                        for &(rate, lbl) in &STANDARD_SAMPLE_RATES {
                                            if toggle_chip(ui, lbl, cur_rate == rate, BRASS)
                                                .clicked()
                                            {
                                                self.config.upsample_mode =
                                                    UpsampleMode::Fixed(rate);
                                                changed = true;
                                            }
                                        }
                                    });
                                    ui.end_row();
                                }

                                ui.label(swiss_kicker("FILTER QUALITY", INK_MUTE));
                                egui::ComboBox::from_id_salt("resampler_quality")
                                    .width(250.0)
                                    .selected_text(self.config.resampler_quality.label())
                                    .show_ui(ui, |ui| {
                                        for q in [
                                            ResamplerQuality::Fast,
                                            ResamplerQuality::High,
                                            ResamplerQuality::Audiophile,
                                            ResamplerQuality::Reference,
                                        ] {
                                            if ui
                                                .selectable_value(
                                                    &mut self.config.resampler_quality,
                                                    q,
                                                    q.label(),
                                                )
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                                ui.end_row();

                                ui.label(swiss_kicker("ROLL-OFF", INK_MUTE));
                                egui::ComboBox::from_id_salt("filter_rolloff")
                                    .width(250.0)
                                    .selected_text(self.config.filter_rolloff.label())
                                    .show_ui(ui, |ui| {
                                        for r in [
                                            FilterRolloff::Sharp,
                                            FilterRolloff::Balanced,
                                            FilterRolloff::Gentle,
                                        ] {
                                            if ui
                                                .selectable_value(
                                                    &mut self.config.filter_rolloff,
                                                    r,
                                                    r.label(),
                                                )
                                                .changed()
                                            {
                                                changed = true;
                                            }
                                        }
                                    });
                                ui.end_row();

                                ui.label(swiss_kicker("HEADROOM", INK_MUTE));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_MD, SPACE_SM);
                                    if ui
                                        .add(
                                            egui::Slider::new(
                                                &mut self.config.headroom_db,
                                                -6.0..=0.0,
                                            )
                                            .suffix(" dB")
                                            .fixed_decimals(1),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    if ui
                                        .checkbox(
                                            &mut self.config.soft_clip,
                                            "Soft-Clip Peak Limiter",
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });
                                ui.end_row();
                            });
                    },
                );

                ui.add_space(SPACE_LG);

                swiss_module(
                    ui,
                    "02",
                    "NATIVE DSD ENGINE · DSF / DFF · DSD64 – DSD512",
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Byte-LUT linear-phase Blackman-Harris FIR decimator or bit-exact DoP v1.1 output.",
                                )
                                .size(12.5)
                                .color(INK_DIM),
                            )
                            .wrap(),
                        );
                        ui.add_space(SPACE_MD);

                        egui::Grid::new("dsd_controls_grid")
                            .num_columns(2)
                            .spacing([SPACE_LG, SPACE_MD])
                            .show(ui, |ui| {
                                ui.label(swiss_kicker("OUTPUT MODE", INK_MUTE));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                                    for mode in [
                                        DsdOutputMode::Pcm88k,
                                        DsdOutputMode::Pcm176k,
                                        DsdOutputMode::Pcm352k,
                                        DsdOutputMode::DoP,
                                    ] {
                                        if toggle_chip(
                                            ui,
                                            mode.label(),
                                            self.config.dsd.output_mode == mode,
                                            BRASS,
                                        )
                                        .clicked()
                                        {
                                            self.config.dsd.output_mode = mode;
                                            changed = true;
                                            auto_vol_changed = true;
                                        }
                                    }
                                });
                                ui.end_row();

                                if self.config.dsd.output_mode != DsdOutputMode::DoP {
                                    ui.label(swiss_kicker("ULTRASONIC LPF", INK_MUTE));
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing =
                                            egui::vec2(SPACE_SM, SPACE_SM);
                                        for &hz in &[24_000u32, 30_000, 40_000, 50_000] {
                                            let lbl = format!("{} kHz", hz / 1000);
                                            if toggle_chip(
                                                ui,
                                                &lbl,
                                                self.config.dsd.cutoff_hz == hz,
                                                SIGNAL,
                                            )
                                            .clicked()
                                            {
                                                self.config.dsd.cutoff_hz = hz;
                                                changed = true;
                                            }
                                        }
                                    });
                                    ui.end_row();

                                    ui.label(swiss_kicker("SACD GAIN", INK_MUTE));
                                    if ui
                                        .checkbox(
                                            &mut self.config.dsd.gain_boost_6db,
                                            "+6 dB SACD Gain Compensation (matches PCM 0 dBFS reference level)",
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                        auto_vol_changed = true;
                                    }
                                    ui.end_row();
                                }
                            });

                        if self.config.dsd.output_mode == DsdOutputMode::DoP {
                            ui.add_space(SPACE_MD);
                            styled_sub_group(ui)
                                .stroke(egui::Stroke::new(1.0_f32, BRASS))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(
                                                "DoP v1.1 packs raw 1-bit DSD frames with 0x05/0xFA markers into bit-exact 24-bit PCM (176.4 kHz for DSD64, 352.8 kHz for DSD128). Volume, EQ, Auto Volume, and upsampling are automatically bypassed.",
                                            )
                                            .size(11.5)
                                            .color(BRASS),
                                        )
                                        .wrap(),
                                    );
                                });
                        }
                    },
                );

                ui.add_space(SPACE_LG);

                swiss_module(
                    ui,
                    "03",
                    "AUTO ADJUST PLAYBACK VOLUME · AUDIOGATE LOUDNESS ENGINE",
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Non-destructive real-time loudness normalization across PCM and DSD tracks (ITU-R BS.1770-4 / EBU R128 + ReplayGain tags). Song mode levels every track individually; Album mode preserves relative dynamics across an album.",
                                )
                                .size(12.5)
                                .color(INK_DIM),
                            )
                            .wrap(),
                        );
                        ui.add_space(SPACE_MD);

                        egui::Grid::new("auto_volume_controls_grid")
                            .num_columns(2)
                            .spacing([SPACE_LG, SPACE_MD])
                            .show(ui, |ui| {
                                ui.label(swiss_kicker("MODE", INK_MUTE));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                                    for (mode, lbl, accent) in [
                                        (AutoVolumeMode::Off, "OFF", INK),
                                        (AutoVolumeMode::Song, "SONG (PER TRACK)", MINT),
                                        (
                                            AutoVolumeMode::Album,
                                            "ALBUM (PRESERVE DYNAMICS)",
                                            BRASS,
                                        ),
                                    ] {
                                        if toggle_chip(
                                            ui,
                                            lbl,
                                            self.config.auto_volume.mode == mode,
                                            accent,
                                        )
                                        .clicked()
                                        {
                                            self.config.auto_volume.mode = mode;
                                            changed = true;
                                            auto_vol_changed = true;
                                        }
                                    }
                                });
                                ui.end_row();

                                ui.label(swiss_kicker("TARGET LEVEL", INK_MUTE));
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = SPACE_SM;
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing =
                                            egui::vec2(SPACE_SM, SPACE_SM);
                                        for &(lufs, lbl) in &[
                                            (-23.0_f32, "-23 LUFS (EBU R128)"),
                                            (-18.0_f32, "-18 LUFS (REPLAYGAIN)"),
                                            (-16.0_f32, "-16 LUFS (AUDIOPHILE)"),
                                            (-14.0_f32, "-14 LUFS (STREAMING)"),
                                        ] {
                                            let is_sel =
                                                (self.config.auto_volume.target_lufs - lufs).abs()
                                                    < 0.15;
                                            if toggle_chip(ui, lbl, is_sel, SIGNAL).clicked() {
                                                self.config.auto_volume.target_lufs = lufs;
                                                changed = true;
                                                auto_vol_changed = true;
                                            }
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut self.config.auto_volume.target_lufs,
                                                    -24.0..=-10.0,
                                                )
                                                .suffix(" LUFS")
                                                .fixed_decimals(1),
                                            )
                                            .changed()
                                        {
                                            changed = true;
                                            auto_vol_changed = true;
                                        }
                                    });
                                });
                                ui.end_row();

                                ui.label(swiss_kicker("PROTECTION", INK_MUTE));
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = SPACE_XS;
                                    if ui
                                        .checkbox(
                                            &mut self.config.auto_volume.prevent_clipping,
                                            "Prevent Clipping (cap positive boost at -1.0 dBFS peak ceiling)",
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                        auto_vol_changed = true;
                                    }
                                    if ui
                                        .checkbox(
                                            &mut self.config.auto_volume.prefer_replaygain_tags,
                                            "Prefer Embedded ReplayGain / R128 Tags when available",
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                        auto_vol_changed = true;
                                    }
                                });
                                ui.end_row();
                            });

                        ui.add_space(SPACE_MD);

                        // Live Loudness Analysis Telemetry Group
                        let av = self.status.auto_volume.clone();
                        let mode = self.config.auto_volume.mode;
                        styled_sub_group(ui)
                            .fill(GROUND)
                            .inner_margin(egui::Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(swiss_kicker("LIVE LOUDNESS TELEMETRY", BRASS));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if mode != AutoVolumeMode::Off
                                                && self.current_track_path().is_some()
                                                && secondary_button(ui, "↻ RE-ANALYZE", None)
                                                    .on_hover_text(
                                                        "Clear cached loudness and freshly scan current track / album",
                                                    )
                                                    .clicked()
                                            {
                                                force_rescan = true;
                                            }
                                        },
                                    );
                                });
                                ui.add_space(SPACE_SM);

                                let (status_txt, status_col) = if mode == AutoVolumeMode::Off {
                                    ("OFF · ORIGINAL MASTER LEVEL".to_string(), INK_MUTE)
                                } else if av.bypassed_for_dop {
                                    ("BYPASSED · BIT-EXACT DoP ACTIVE".to_string(), BRASS)
                                } else if av.analyzing {
                                    (
                                        format!(
                                            "ANALYZING {} LOUDNESS…",
                                            mode.short_label()
                                        ),
                                        SIGNAL,
                                    )
                                } else if av.active {
                                    (
                                        format!("ACTIVE · {} MODE", mode.short_label()),
                                        MINT,
                                    )
                                } else {
                                    ("READY · LOAD A TRACK".to_string(), INK_MUTE)
                                };

                                let meas_txt = match (av.measured_lufs, av.peak_dbfs) {
                                    (Some(lufs), Some(pk)) => {
                                        format!("{:.1} LUFS · PEAK {:.1} dBFS", lufs, pk)
                                    }
                                    (Some(lufs), None) => format!("{:.1} LUFS", lufs),
                                    _ => "—".to_string(),
                                };

                                let (gain_txt, gain_col) = if mode == AutoVolumeMode::Off
                                    || av.bypassed_for_dop
                                    || !av.active
                                {
                                    ("0.0 dB".to_string(), INK_MUTE)
                                } else if av.peak_limited {
                                    (
                                        format!("{:+.1} dB (PEAK SAFE)", av.applied_gain_db),
                                        SIGNAL,
                                    )
                                } else if av.applied_gain_db > 0.05 {
                                    (format!("{:+.1} dB", av.applied_gain_db), BRASS)
                                } else {
                                    (format!("{:+.1} dB", av.applied_gain_db), MINT)
                                };

                                egui::Grid::new("auto_vol_telemetry_grid")
                                    .num_columns(2)
                                    .spacing([SPACE_LG, SPACE_XS])
                                    .show(ui, |ui| {
                                        ui.label(swiss_kicker("STATUS", INK_MUTE));
                                        ui.label(
                                            egui::RichText::new(status_txt)
                                                .size(11.0)
                                                .monospace()
                                                .strong()
                                                .color(status_col),
                                        );
                                        ui.end_row();

                                        ui.label(swiss_kicker("MEASURED", INK_MUTE));
                                        ui.label(
                                            egui::RichText::new(meas_txt)
                                                .size(11.0)
                                                .monospace()
                                                .color(INK),
                                        );
                                        ui.end_row();

                                        ui.label(swiss_kicker("APPLIED GAIN", INK_MUTE));
                                        ui.label(
                                            egui::RichText::new(gain_txt)
                                                .size(11.5)
                                                .monospace()
                                                .strong()
                                                .color(gain_col),
                                        );
                                        ui.end_row();

                                        ui.label(swiss_kicker("SOURCE", INK_MUTE));
                                        ui.label(
                                            egui::RichText::new(&av.source_label)
                                                .size(11.0)
                                                .monospace()
                                                .color(INK_DIM),
                                        );
                                        ui.end_row();
                                    });
                            });
                    },
                );

                ui.add_space(SPACE_MD);
                if changed {
                    self.push_config();
                }
                if force_rescan {
                    self.trigger_auto_volume(None, true);
                    self.show_toast("RE-ANALYZING LOUDNESS (ITU-R BS.1770-4)", false);
                } else if auto_vol_changed {
                    self.trigger_auto_volume(None, false);
                }
            });
    }

    fn render_multichannel_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let mut changed = false;

                swiss_module(
                    ui,
                    "01",
                    "MULTI-CHANNEL ROUTING & ITU-R BS.775 DOWNMIX MATRIX",
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Native passthrough or normalized stereo / headphone spatial downmix across 1.0, 2.0, 3.0, 4.0, 5.0, 5.1, 6.1, and 7.1 PCM & DSD streams.",
                                )
                                .size(12.5)
                                .color(INK_DIM),
                            )
                            .wrap(),
                        );
                        ui.add_space(SPACE_MD);

                        egui::Grid::new("downmix_grid")
                            .num_columns(2)
                            .spacing([SPACE_LG, SPACE_MD])
                            .show(ui, |ui| {
                                ui.label(swiss_kicker("ROUTING MODE", INK_MUTE));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                                    for mode in [
                                        DownmixMode::Auto,
                                        DownmixMode::NativePassthrough,
                                        DownmixMode::StereoItu,
                                        DownmixMode::HeadphoneSpatial,
                                    ] {
                                        if toggle_chip(
                                            ui,
                                            mode.label(),
                                            self.config.multichannel.downmix_mode == mode,
                                            SIGNAL,
                                        )
                                        .clicked()
                                        {
                                            self.config.multichannel.downmix_mode = mode;
                                            changed = true;
                                        }
                                    }
                                });
                                ui.end_row();

                                ui.label(swiss_kicker("CENTER MIX (ITU −3 dB)", INK_MUTE));
                                if ui
                                    .add(
                                        egui::Slider::new(
                                            &mut self.config.multichannel.center_gain_db,
                                            -9.0..=3.0,
                                        )
                                        .suffix(" dB")
                                        .fixed_decimals(1),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.end_row();

                                ui.label(swiss_kicker("SURROUND MIX (ITU −3 dB)", INK_MUTE));
                                if ui
                                    .add(
                                        egui::Slider::new(
                                            &mut self.config.multichannel.surround_gain_db,
                                            -12.0..=3.0,
                                        )
                                        .suffix(" dB")
                                        .fixed_decimals(1),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.end_row();

                                if ui
                                    .checkbox(
                                        &mut self.config.multichannel.include_lfe_in_downmix,
                                        "Include LFE (.1 Subwoofer)",
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .add_enabled(
                                        self.config.multichannel.include_lfe_in_downmix,
                                        egui::Slider::new(
                                            &mut self.config.multichannel.lfe_gain_db,
                                            -15.0..=0.0,
                                        )
                                        .suffix(" dB")
                                        .fixed_decimals(1),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.end_row();

                                ui.label(swiss_kicker("SPATIAL CROSSFEED", INK_MUTE));
                                if ui
                                    .add(
                                        egui::Slider::new(
                                            &mut self.config.multichannel.crossfeed_amount,
                                            0.0..=0.50,
                                        )
                                        .fixed_decimals(2),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.end_row();
                            });
                    },
                );

                ui.add_space(SPACE_LG);

                let (ch_count, labels) = if let Some(ref info) = self.track_info {
                    (
                        info.channels.clamp(1, MAX_METER_CHANNELS as u16) as usize,
                        info.channel_labels.clone(),
                    )
                } else {
                    (
                        6,
                        vec![
                            "FL".into(),
                            "FR".into(),
                            "FC".into(),
                            "LFE".into(),
                            "SL".into(),
                            "SR".into(),
                        ],
                    )
                };

                let mut ms_changed = false;
                swiss_module(ui, "02", "DISCRETE CHANNEL MIXER · MUTE / SOLO", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(
                                "Isolate (S) or Mute (M) individual surround channels in real time.",
                            )
                            .size(12.0)
                            .color(INK_DIM),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if secondary_button(ui, "RESET ALL", None).clicked() {
                                self.mute_solo = ChannelMuteSolo::default();
                                ms_changed = true;
                            }
                        });
                    });
                    ui.add_space(SPACE_MD);

                    styled_sub_group(ui).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::horizontal()
                            .id_salt("multichannel_mixer_scroll")
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                                    for ch in 0..ch_count {
                                        let lbl = labels
                                            .get(ch)
                                            .cloned()
                                            .unwrap_or_else(|| format!("CH{}", ch + 1));
                                        let peak = self.smooth_peaks[ch];
                                        let rms = self.smooth_rms[ch];
                                        let hold = self.peak_hold[ch];

                                        egui::Frame::group(ui.style())
                                            .fill(SURFACE)
                                            .corner_radius(egui::CornerRadius::same(5))
                                            .stroke(egui::Stroke::new(1.0_f32, RULE))
                                            .inner_margin(egui::Margin::symmetric(12, 12))
                                            .show(ui, |ui| {
                                                ui.set_min_width(68.0);
                                                ui.vertical_centered(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "CH {:02}",
                                                            ch + 1
                                                        ))
                                                        .size(9.0)
                                                        .monospace()
                                                        .color(INK_MUTE),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(&lbl)
                                                            .size(12.5)
                                                            .strong()
                                                            .monospace()
                                                            .color(INK),
                                                    );
                                                    ui.add_space(SPACE_SM);
                                                    paint_vertical_vu(
                                                        ui, 24.0, 148.0, peak, rms, hold,
                                                    );
                                                    ui.add_space(SPACE_SM);
                                                    let db = linear_to_db(peak);
                                                    ui.label(
                                                        egui::RichText::new(if db <= -59.5 {
                                                            "-∞".into()
                                                        } else {
                                                            format!("{:.0}dB", db)
                                                        })
                                                        .size(10.0)
                                                        .monospace()
                                                        .color(INK_MUTE),
                                                    );
                                                    ui.add_space(SPACE_SM);
                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = SPACE_XS;
                                                        let m_active = self.mute_solo.muted[ch];
                                                        if toggle_chip(ui, "M", m_active, ALERT)
                                                            .clicked()
                                                        {
                                                            self.mute_solo.muted[ch] = !m_active;
                                                            ms_changed = true;
                                                        }
                                                        let s_active = self.mute_solo.solo[ch];
                                                        if toggle_chip(ui, "S", s_active, BRASS)
                                                            .clicked()
                                                        {
                                                            self.mute_solo.solo[ch] = !s_active;
                                                            ms_changed = true;
                                                        }
                                                    });
                                                });
                                            });
                                    }
                                });
                            });
                    });
                });

                ui.add_space(SPACE_MD);
                if changed {
                    self.push_config();
                }
                if ms_changed {
                    self.engine
                        .send(AudioCommand::SetChannelMuteSolo(self.mute_solo.clone()));
                }
            });
    }

    fn render_eq_tab(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                swiss_module(ui, "01", "10-BAND 64-BIT BIQUAD EQUALIZER", |ui| {
                    egui::Grid::new("eq_controls_grid")
                        .num_columns(2)
                        .spacing([SPACE_LG, SPACE_SM])
                        .show(ui, |ui| {
                            ui.label(swiss_kicker("MASTER EQ", INK_MUTE));
                            let eq_on = self.config.eq.enabled;
                            if ui
                                .checkbox(
                                    &mut self.config.eq.enabled,
                                    egui::RichText::new(if eq_on {
                                        "ENGAGED (64-BIT BIQUAD ACTIVE)"
                                    } else {
                                        "BYPASSED (BIT-TRANSPARENT)"
                                    })
                                    .size(11.0)
                                    .monospace()
                                    .strong()
                                    .color(if eq_on { SIGNAL } else { INK_DIM }),
                                )
                                .changed()
                            {
                                changed = true;
                            }
                            ui.end_row();

                            ui.label(swiss_kicker("PRESET", INK_MUTE));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
                                egui::ComboBox::from_id_salt("eq_preset_box")
                                    .width(185.0)
                                    .selected_text(&self.config.eq.preset_name)
                                    .show_ui(ui, |ui| {
                                        for &(name, preamp, bands) in EqSettings::presets() {
                                            if ui
                                                .selectable_label(
                                                    self.config.eq.preset_name == name,
                                                    name,
                                                )
                                                .clicked()
                                            {
                                                self.config.eq.preset_name = name.to_string();
                                                self.config.eq.preamp_db = preamp;
                                                self.config.eq.bands_db = bands;
                                                self.config.eq.enabled = name != "Flat";
                                                changed = true;
                                            }
                                        }
                                    });
                                if secondary_button(ui, "RESET FLAT", None).clicked() {
                                    self.config.eq.preamp_db = 0.0;
                                    self.config.eq.bands_db = [0.0; 10];
                                    self.config.eq.preset_name = "Flat".into();
                                    changed = true;
                                }
                            });
                            ui.end_row();
                        });

                    ui.add_space(SPACE_MD);

                    styled_sub_group(ui)
                        .inner_margin(egui::Margin::symmetric(16, 18))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::horizontal()
                                .id_salt("eq_fader_rack_scroll")
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = SPACE_SM;

                                        // Preamp column
                                        ui.allocate_ui(egui::vec2(50.0, 206.0), |ui| {
                                            ui.vertical_centered(|ui| {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{:+.1}",
                                                        self.config.eq.preamp_db
                                                    ))
                                                    .size(10.5)
                                                    .monospace()
                                                    .color(BRASS),
                                                );
                                                ui.add_space(SPACE_SM);
                                                if ui
                                                    .add(
                                                        egui::Slider::new(
                                                            &mut self.config.eq.preamp_db,
                                                            -12.0..=12.0,
                                                        )
                                                        .vertical()
                                                        .show_value(false),
                                                    )
                                                    .changed()
                                                {
                                                    self.config.eq.preset_name = "Custom".into();
                                                    changed = true;
                                                }
                                                ui.add_space(SPACE_SM);
                                                ui.label(
                                                    egui::RichText::new("PRE")
                                                        .size(10.5)
                                                        .monospace()
                                                        .strong()
                                                        .color(BRASS),
                                                );
                                            });
                                        });

                                        v_hairline(ui, 186.0);

                                        for (i, &freq_label) in EQ_LABELS.iter().enumerate() {
                                            ui.allocate_ui(egui::vec2(48.0, 206.0), |ui| {
                                                ui.vertical_centered(|ui| {
                                                    let val = self.config.eq.bands_db[i];
                                                    ui.label(
                                                        egui::RichText::new(format!("{:+.1}", val))
                                                            .size(10.5)
                                                            .monospace()
                                                            .color(if val.abs() > 0.1 {
                                                                SIGNAL
                                                            } else {
                                                                INK_MUTE
                                                            }),
                                                    );
                                                    ui.add_space(SPACE_SM);
                                                    if ui
                                                        .add(
                                                            egui::Slider::new(
                                                                &mut self.config.eq.bands_db[i],
                                                                -12.0..=12.0,
                                                            )
                                                            .vertical()
                                                            .show_value(false),
                                                        )
                                                        .changed()
                                                    {
                                                        self.config.eq.preset_name =
                                                            "Custom".into();
                                                        self.config.eq.enabled = true;
                                                        changed = true;
                                                    }
                                                    ui.add_space(SPACE_SM);
                                                    ui.label(
                                                        egui::RichText::new(freq_label)
                                                            .size(10.5)
                                                            .monospace()
                                                            .color(INK_DIM),
                                                    );
                                                });
                                            });
                                        }
                                    });
                                });
                        });
                });
            });

        if changed {
            self.push_config();
        }
    }

    fn render_bottom_transport(&mut self, ctx: &egui::Context) {
        let compact = ctx.screen_rect().width() < 920.0;

        egui::TopBottomPanel::bottom("transport_deck")
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin::symmetric(20, 12))
                    .stroke(egui::Stroke::new(1.0_f32, RULE)),
            )
            .show(ctx, |ui| {
                // Row 1: Precision Custom Tactile Scrub Rail
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_MD;
                    let pos = if self.seeking {
                        self.seek_preview
                    } else {
                        self.status.position
                    };
                    let dur = self.status.duration.max(0.0);

                    ui.add_sized(
                        egui::vec2(46.0, 18.0),
                        egui::Label::new(
                            egui::RichText::new(format_time(pos))
                                .monospace()
                                .size(11.0)
                                .color(INK),
                        ),
                    );

                    let rail_w = (ui.available_width() - 62.0).max(120.0);
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(rail_w, 18.0),
                        egui::Sense::click_and_drag(),
                    );
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    let frac = if dur > 0.0 {
                        (pos / dur).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    let cy = rect.center().y;
                    let p = ui.painter();

                    p.line_segment(
                        [egui::pos2(rect.left(), cy), egui::pos2(rect.right(), cy)],
                        egui::Stroke::new(2.0_f32, RULE),
                    );
                    let head_x = rect.left() + rect.width() * frac;
                    if frac > 0.0 {
                        p.line_segment(
                            [egui::pos2(rect.left(), cy), egui::pos2(head_x, cy)],
                            egui::Stroke::new(2.0_f32, SIGNAL),
                        );
                    }
                    p.rect_filled(
                        egui::Rect::from_center_size(egui::pos2(head_x, cy), egui::vec2(4.0, 12.0)),
                        egui::CornerRadius::same(1),
                        INK,
                    );

                    if let Some(pointer) = resp.interact_pointer_pos() {
                        let new_frac =
                            ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
                        let target_sec = new_frac * dur;
                        if resp.dragged() {
                            self.seeking = true;
                            self.seek_preview = target_sec;
                        } else if resp.clicked() && dur > 0.0 {
                            self.engine.send(AudioCommand::Seek(target_sec));
                        }
                    }
                    if resp.drag_stopped() {
                        self.seeking = false;
                        if dur > 0.0 {
                            self.engine.send(AudioCommand::Seek(self.seek_preview));
                        }
                    }

                    ui.add_sized(
                        egui::vec2(46.0, 18.0),
                        egui::Label::new(
                            egui::RichText::new(format_time(dur))
                                .monospace()
                                .size(11.0)
                                .color(INK_MUTE),
                        ),
                    );
                });

                ui.add_space(SPACE_SM);

                // Row 2: Orthogonal Hardware Transport Deck with clear primary/secondary/ghost hierarchy
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_SM;

                    if ghost_button(
                        ui,
                        "SHUF",
                        if self.config.shuffle {
                            Some(SIGNAL)
                        } else {
                            None
                        },
                    )
                    .on_hover_text("Toggle Shuffle")
                    .clicked()
                    {
                        self.config.shuffle = !self.config.shuffle;
                        self.config.save();
                    }

                    if secondary_button(ui, "⏮", None)
                        .on_hover_text("Previous Track (P)")
                        .clicked()
                    {
                        self.play_prev();
                    }

                    let playing = self.status.state == PlaybackState::Playing;
                    let play_label = if playing { "⏸ PAUSE" } else { "▶ PLAY" };
                    if ui
                        .add_sized(
                            egui::vec2(96.0, 32.0),
                            egui::Button::new(
                                egui::RichText::new(play_label)
                                    .size(11.5)
                                    .monospace()
                                    .strong()
                                    .color(GROUND),
                            )
                            .fill(if playing { INK } else { SIGNAL })
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                if playing { INK } else { SIGNAL },
                            ))
                            .corner_radius(egui::CornerRadius::same(4)),
                        )
                        .on_hover_text("Play / Pause (Space)")
                        .clicked()
                    {
                        self.toggle_play_pause();
                    }

                    if secondary_button(ui, "⏹", None)
                        .on_hover_text("Stop")
                        .clicked()
                    {
                        self.engine.send(AudioCommand::Stop);
                    }

                    if secondary_button(ui, "⏭", None)
                        .on_hover_text("Next Track (N)")
                        .clicked()
                    {
                        self.play_next(true);
                    }

                    let (rep_lbl, rep_accent) = match self.config.repeat_mode {
                        RepeatMode::Off => ("RPT", None),
                        RepeatMode::All => ("RPT ALL", Some(SIGNAL)),
                        RepeatMode::One => ("RPT 1", Some(BRASS)),
                    };
                    if ghost_button(ui, rep_lbl, rep_accent)
                        .on_hover_text("Cycle Repeat: Off / All / One")
                        .clicked()
                    {
                        self.config.repeat_mode = match self.config.repeat_mode {
                            RepeatMode::Off => RepeatMode::All,
                            RepeatMode::All => RepeatMode::One,
                            RepeatMode::One => RepeatMode::Off,
                        };
                        self.config.save();
                    }

                    v_hairline(ui, 22.0);

                    // Volume Controls
                    let mute_lbl = if self.muted || self.config.volume <= 0.001 {
                        "MUTE"
                    } else {
                        "VOL"
                    };
                    if ghost_button(
                        ui,
                        mute_lbl,
                        if self.muted { Some(ALERT) } else { None },
                    )
                    .on_hover_text("Mute / Unmute (M)")
                    .clicked()
                    {
                        self.muted = !self.muted;
                        self.engine.send(AudioCommand::SetMuted(self.muted));
                    }

                    ui.style_mut().spacing.slider_width = if compact { 82.0 } else { 116.0 };
                    let vol_enabled = !self.status.is_dop;
                    if ui
                        .add_enabled(
                            vol_enabled,
                            egui::Slider::new(&mut self.config.volume, 0.0..=1.0)
                                .show_value(false)
                                .trailing_fill(true),
                        )
                        .changed()
                    {
                        self.muted = false;
                        self.engine.send(AudioCommand::SetMuted(false));
                        self.engine
                            .send(AudioCommand::SetVolume(self.config.volume));
                        self.config.save();
                    }
                    let vol_text = if self.status.is_dop {
                        "DOP 0.0dB".to_string()
                    } else if self.muted || self.config.volume <= 0.001 {
                        "-∞ dB".to_string()
                    } else {
                        let db = 20.0 * self.config.volume.log10();
                        format!("{:+.1} dB", db)
                    };
                    ui.label(
                        egui::RichText::new(vol_text)
                            .size(10.5)
                            .monospace()
                            .color(INK_DIM),
                    );

                    v_hairline(ui, 18.0);

                    // Quick 1-click KORG AudioGate Auto Adjust Playback Volume cycle control
                    let av_mode = self.config.auto_volume.mode;
                    let (av_chip_lbl, av_chip_col) = match av_mode {
                        AutoVolumeMode::Off => ("AUTO VOL", None),
                        AutoVolumeMode::Song => ("AUTO · SONG", Some(MINT)),
                        AutoVolumeMode::Album => ("AUTO · ALBUM", Some(BRASS)),
                    };
                    let av_tip = match av_mode {
                        AutoVolumeMode::Off => format!(
                            "Auto Adjust Playback Volume: Off\nClick to cycle Off → Song → Album (Target {:.1} LUFS)",
                            self.config.auto_volume.target_lufs
                        ),
                        _ => {
                            let av = &self.status.auto_volume;
                            let detail = if av.bypassed_for_dop {
                                "Bypassed for DoP".to_string()
                            } else if av.analyzing {
                                "Analyzing ITU-R BS.1770-4 loudness…".to_string()
                            } else if let Some(lufs) = av.measured_lufs {
                                format!(
                                    "Measured {:.1} LUFS · Applied {:+.1} dB ({})",
                                    lufs, av.applied_gain_db, av.source_label
                                )
                            } else {
                                "Ready".to_string()
                            };
                            format!(
                                "Auto Adjust Playback Volume: {}\n{}\nClick to cycle Off → Song → Album",
                                av_mode.label(),
                                detail
                            )
                        }
                    };
                    if ghost_button(ui, av_chip_lbl, av_chip_col)
                        .on_hover_text(av_tip)
                        .clicked()
                    {
                        self.cycle_auto_volume_mode();
                    }
                    if av_mode != AutoVolumeMode::Off && !compact {
                        let (offset_txt, offset_col) = if self.status.auto_volume.bypassed_for_dop {
                            ("DoP".to_string(), INK_MUTE)
                        } else if self.status.auto_volume.analyzing {
                            ("SCAN…".to_string(), BRASS)
                        } else if self.status.auto_volume.active {
                            (
                                format!("{:+.1} dB", self.status.auto_volume.applied_gain_db),
                                if self.status.auto_volume.peak_limited {
                                    SIGNAL
                                } else if self.status.auto_volume.applied_gain_db > 0.05 {
                                    BRASS
                                } else {
                                    MINT
                                },
                            )
                        } else {
                            ("0.0 dB".to_string(), INK_MUTE)
                        };
                        ui.label(
                            egui::RichText::new(offset_txt)
                                .size(10.0)
                                .monospace()
                                .strong()
                                .color(offset_col),
                        );
                    }

                    // Right-aligned grouped telemetry readout
                    if !compact && self.status.output_sample_rate > 0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_telemetry_block(ui);
                        });
                    }
                });

                if compact && self.status.output_sample_rate > 0 {
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        self.render_telemetry_block(ui);
                    });
                }
            });
    }

    fn render_telemetry_block(&self, ui: &mut egui::Ui) {
        let ch_str = if self.status.source_channels != self.status.output_channels {
            format!(
                "{}→{}CH",
                self.status.source_channels, self.status.output_channels
            )
        } else {
            format!("{}CH", self.status.output_channels)
        };
        let tele = format!(
            "{} · {} · {}",
            format_rate(self.status.output_sample_rate).to_uppercase(),
            self.status.output_format_label.to_uppercase(),
            ch_str
        );
        egui::Frame::group(ui.style())
            .fill(GROUND)
            .corner_radius(egui::CornerRadius::same(4))
            .stroke(egui::Stroke::new(1.0_f32, RULE))
            .inner_margin(egui::Margin::symmetric(12, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                    let led_col = if self.status.state == PlaybackState::Playing {
                        if self.status.is_dsd {
                            BRASS
                        } else {
                            MINT
                        }
                    } else {
                        INK_MUTE
                    };
                    ui.label(egui::RichText::new("●").size(8.0).color(led_col));
                    ui.label(
                        egui::RichText::new(tele)
                            .size(10.5)
                            .monospace()
                            .color(INK),
                    )
                    .on_hover_text(format!("Active Output: {}", self.status.output_device_name));
                });
            });
    }

    // ── Precision Painted Instruments ────────────────────────────────────────

    fn paint_spectrum_analyzer(&self, ui: &mut egui::Ui, height: f32) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let p = ui.painter_at(rect);

        p.rect_filled(rect, egui::CornerRadius::same(5), WELL);
        p.rect_stroke(
            rect,
            egui::CornerRadius::same(5),
            egui::Stroke::new(1.0_f32, RULE),
            egui::StrokeKind::Inside,
        );

        for (idx, lbl) in [(1, "-12"), (2, "-24"), (3, "-48")] {
            let y = rect.top() + rect.height() * (idx as f32 / 4.2);
            p.line_segment(
                [
                    egui::pos2(rect.left() + 2.0, y),
                    egui::pos2(rect.right() - 2.0, y),
                ],
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(20, 20, 25)),
            );
            p.text(
                egui::pos2(rect.left() + 6.0, y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                lbl,
                egui::FontId::monospace(8.5),
                INK_MUTE,
            );
        }

        let plot = rect.shrink2(egui::vec2(28.0, 14.0));
        let bar_slot = plot.width() / SPECTRUM_BANDS as f32;
        let bar_w = (bar_slot * 0.62).max(2.0);
        let y_bottom = plot.bottom() - 10.0;
        let max_h = (plot.height() - 12.0).max(10.0);

        for i in 0..SPECTRUM_BANDS {
            let val = self.spectrum.bands[i].clamp(0.0, 1.0);
            let pk = self.spectrum.peaks[i].clamp(0.0, 1.0);
            let x0 = plot.left() + i as f32 * bar_slot + (bar_slot - bar_w) * 0.5;
            let bar_h = val * max_h;
            let y_top = y_bottom - bar_h;

            let col = if val > 0.78 {
                SIGNAL
            } else if val > 0.45 {
                BRASS
            } else {
                egui::Color32::from_rgb(172, 172, 178)
            };

            if bar_h > 1.0 {
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x0, y_top),
                        egui::pos2(x0 + bar_w, y_bottom),
                    ),
                    egui::CornerRadius::same(1),
                    col,
                );
            }

            let pk_y = y_bottom - pk * max_h;
            if pk > 0.02 {
                p.line_segment(
                    [egui::pos2(x0, pk_y), egui::pos2(x0 + bar_w, pk_y)],
                    egui::Stroke::new(1.5_f32, SIGNAL),
                );
            }
        }

        let freq_marks = [
            (0.02, "30HZ"),
            (0.22, "120HZ"),
            (0.44, "500HZ"),
            (0.60, "1.5KHZ"),
            (0.78, "6KHZ"),
            (0.96, "20KHZ"),
        ];
        for (frac, lbl) in freq_marks {
            p.text(
                egui::pos2(plot.left() + plot.width() * frac, rect.bottom() - 4.0),
                egui::Align2::CENTER_BOTTOM,
                lbl,
                egui::FontId::monospace(9.0),
                INK_MUTE,
            );
        }
    }

    fn paint_horizontal_channel_meters(
        &self,
        ui: &mut egui::Ui,
        ch_count: usize,
        labels: &[String],
    ) {
        ui.spacing_mut().item_spacing.y = SPACE_XS + 1.0;
        for ch in 0..ch_count {
            let lbl = labels
                .get(ch)
                .cloned()
                .unwrap_or_else(|| format!("CH{}", ch + 1));
            let peak = self.smooth_peaks[ch];
            let rms = self.smooth_rms[ch];
            let hold = self.peak_hold[ch];

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_SM;
                ui.add_sized(
                    egui::vec2(34.0, 14.0),
                    egui::Label::new(
                        egui::RichText::new(format!("{:<4}", lbl))
                            .size(10.5)
                            .monospace()
                            .strong()
                            .color(INK_DIM),
                    ),
                );
                let meter_w = (ui.available_width() - 60.0).max(80.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(meter_w, 10.0), egui::Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, egui::CornerRadius::same(2), WELL);
                p.rect_stroke(
                    rect,
                    egui::CornerRadius::same(2),
                    egui::Stroke::new(1.0_f32, RULE),
                    egui::StrokeKind::Inside,
                );

                let peak_norm = db_to_meter_norm(linear_to_db(peak));
                let rms_norm = db_to_meter_norm(linear_to_db(rms));
                let hold_norm = db_to_meter_norm(linear_to_db(hold));

                if peak_norm > 0.005 {
                    let col = if peak_norm > 0.92 {
                        ALERT
                    } else if peak_norm > 0.75 {
                        SIGNAL
                    } else {
                        BRASS
                    };
                    p.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 1.0, rect.top() + 1.0),
                            egui::pos2(
                                rect.left() + 1.0 + (rect.width() - 2.0) * peak_norm,
                                rect.bottom() - 1.0,
                            ),
                        ),
                        egui::CornerRadius::same(1),
                        col.linear_multiply(0.55),
                    );
                }

                if rms_norm > 0.005 {
                    p.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 1.0, rect.top() + 2.0),
                            egui::pos2(
                                rect.left() + 1.0 + (rect.width() - 2.0) * rms_norm,
                                rect.bottom() - 2.0,
                            ),
                        ),
                        egui::CornerRadius::same(1),
                        MINT,
                    );
                }

                if hold_norm > 0.01 {
                    let hx = rect.left() + 1.0 + (rect.width() - 2.0) * hold_norm;
                    p.line_segment(
                        [
                            egui::pos2(hx, rect.top() + 1.0),
                            egui::pos2(hx, rect.bottom() - 1.0),
                        ],
                        egui::Stroke::new(2.0_f32, INK),
                    );
                }

                let db = linear_to_db(peak);
                ui.add_sized(
                    egui::vec2(46.0, 14.0),
                    egui::Label::new(
                        egui::RichText::new(if db <= -59.5 {
                            "  -∞ ".to_string()
                        } else {
                            format!("{:>4.0}dB", db)
                        })
                        .size(10.0)
                        .monospace()
                        .color(INK_MUTE),
                    ),
                );
            });
        }
    }
}

// ── Font Pairing & Swiss Theme Configuration ─────────────────────────────────

fn install_editorial_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let serif_candidates = [
        r"C:\Windows\Fonts\georgia.ttf",
        r"C:\Windows\Fonts\georgiab.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    ];
    let mut serif_chain = Vec::new();
    for path in serif_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("sys_serif".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            serif_chain.push("sys_serif".to_owned());
            break;
        }
    }
    if let Some(default_prop) = fonts.families.get(&egui::FontFamily::Proportional) {
        serif_chain.extend(default_prop.iter().cloned());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name(EDITORIAL_FONT.into()), serif_chain);

    let sans_candidates = [
        r"C:\Windows\Fonts\bahnschrift.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    for path in sans_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("sys_sans".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "sys_sans".to_owned());
            break;
        }
    }

    let mono_candidates = [
        r"C:\Windows\Fonts\CascadiaMono.ttf",
        r"C:\Windows\Fonts\consola.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ];
    for path in mono_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("sys_mono".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "sys_mono".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);
}

fn apply_swiss_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = SURFACE;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = WELL;
    visuals.faint_bg_color = SURFACE_ALT;
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.window_stroke = egui::Stroke::new(1.0_f32, RULE_STRONG);
    visuals.window_corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, RULE);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(4);

    visuals.widgets.inactive.bg_fill = SURFACE_ALT;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, RULE);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);

    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(28, 28, 34);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, RULE_STRONG);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);

    visuals.widgets.active.bg_fill = SIGNAL;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, SIGNAL);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);

    visuals.selection.bg_fill = SIGNAL;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, INK);
    visuals.override_text_color = Some(INK);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
    style.spacing.button_padding = egui::vec2(11.0, 6.0);
    ctx.set_style(style);
}

// ── Reusable Styled Groups, Typography & Button Hierarchy ────────────────────

fn editorial_text(text: &str, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text)
        .font(egui::FontId::new(
            size,
            egui::FontFamily::Name(EDITORIAL_FONT.into()),
        ))
        .color(color)
}

fn swiss_kicker(text: &str, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text)
        .size(10.0)
        .monospace()
        .strong()
        .color(color)
}

/// Primary outer group block built on `egui::Frame::group(ui.style())` with subtle rounding & generous padding.
fn styled_group(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::group(ui.style())
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, RULE))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(20, 18))
}

/// Nested recessed control/instrument group built on `egui::Frame::group(ui.style())`.
fn styled_sub_group(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::group(ui.style())
        .fill(WELL)
        .stroke(egui::Stroke::new(1.0_f32, RULE))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(14, 14))
}

fn swiss_module(
    ui: &mut egui::Ui,
    index: &str,
    title: &str,
    add_body: impl FnOnce(&mut egui::Ui),
) {
    styled_group(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_SM;
            ui.label(
                egui::RichText::new(index)
                    .size(10.0)
                    .monospace()
                    .strong()
                    .color(SIGNAL),
            );
            ui.label(
                egui::RichText::new("/")
                    .size(10.0)
                    .monospace()
                    .color(INK_MUTE),
            );
            ui.label(swiss_kicker(title, INK_DIM));
        });
        ui.add_space(SPACE_SM);
        h_hairline(ui, RULE);
        ui.add_space(SPACE_MD);
        add_body(ui);
    });
}

/// Primary CTA button: solid high-contrast accent fill with crisp dark label.
fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(10.5)
                .monospace()
                .strong()
                .color(GROUND),
        )
        .fill(SIGNAL)
        .stroke(egui::Stroke::new(1.0_f32, SIGNAL))
        .corner_radius(egui::CornerRadius::same(4)),
    )
}

/// Secondary action button: structured surface fill with 1px hairline border.
fn secondary_button(
    ui: &mut egui::Ui,
    text: &str,
    accent: Option<egui::Color32>,
) -> egui::Response {
    let fg = accent.unwrap_or(INK);
    let border = accent.unwrap_or(RULE_STRONG);
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .size(10.5)
                .monospace()
                .strong()
                .color(fg),
        )
        .fill(SURFACE_ALT)
        .stroke(egui::Stroke::new(1.0_f32, border))
        .corner_radius(egui::CornerRadius::same(4)),
    )
}

/// Tertiary / utility ghost button: low-noise surface for compact controls.
fn ghost_button(
    ui: &mut egui::Ui,
    text: &str,
    accent: Option<egui::Color32>,
) -> egui::Response {
    let fg = accent.unwrap_or(INK_DIM);
    let border = accent.unwrap_or(RULE);
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(10.0).monospace().color(fg))
            .fill(SURFACE)
            .stroke(egui::Stroke::new(1.0_f32, border))
            .corner_radius(egui::CornerRadius::same(4)),
    )
}

/// Navigation tab button with clear active vs inactive visual hierarchy.
fn nav_tab_button(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let (fg, fill, border) = if active {
        (INK, SURFACE_ALT, SIGNAL)
    } else {
        (INK_DIM, egui::Color32::TRANSPARENT, RULE)
    };
    let mut rt = egui::RichText::new(label).size(10.5).monospace().color(fg);
    if active {
        rt = rt.strong();
    }
    ui.add(
        egui::Button::new(rt)
            .fill(fill)
            .stroke(egui::Stroke::new(1.0_f32, border))
            .corner_radius(egui::CornerRadius::same(4)),
    )
}

/// Selectable option chip with unmistakable selected vs unselected states.
fn toggle_chip(
    ui: &mut egui::Ui,
    text: &str,
    selected: bool,
    accent: egui::Color32,
) -> egui::Response {
    let (fg, fill, border) = if selected {
        (GROUND, accent, accent)
    } else {
        (INK_DIM, SURFACE_ALT, RULE)
    };
    let mut rt = egui::RichText::new(text).size(10.5).monospace().color(fg);
    if selected {
        rt = rt.strong();
    }
    ui.add(
        egui::Button::new(rt)
            .fill(fill)
            .stroke(egui::Stroke::new(1.0_f32, border))
            .corner_radius(egui::CornerRadius::same(4)),
    )
}

fn stamp_tag(ui: &mut egui::Ui, text: &str, fg: egui::Color32, border: egui::Color32) {
    egui::Frame::group(ui.style())
        .fill(SURFACE)
        .corner_radius(egui::CornerRadius::same(3))
        .stroke(egui::Stroke::new(1.0_f32, border))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(9.5)
                    .monospace()
                    .strong()
                    .color(fg),
            );
        });
}

fn h_hairline(ui: &mut egui::Ui, color: egui::Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.center().y),
            egui::pos2(rect.right(), rect.center().y),
        ],
        egui::Stroke::new(1.0_f32, color),
    );
}

fn v_hairline(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.center().x, rect.top()),
            egui::pos2(rect.center().x, rect.bottom()),
        ],
        egui::Stroke::new(1.0_f32, RULE),
    );
}

fn paint_vertical_vu(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    peak: f32,
    rms: f32,
    hold: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, egui::CornerRadius::same(3), WELL);
    p.rect_stroke(
        rect,
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.0_f32, RULE),
        egui::StrokeKind::Inside,
    );

    let peak_n = db_to_meter_norm(linear_to_db(peak));
    let rms_n = db_to_meter_norm(linear_to_db(rms));
    let hold_n = db_to_meter_norm(linear_to_db(hold));
    let inner_h = (rect.height() - 2.0).max(1.0);

    if peak_n > 0.005 {
        let col = if peak_n > 0.92 {
            ALERT
        } else if peak_n > 0.75 {
            SIGNAL
        } else {
            BRASS
        };
        let top = rect.bottom() - 1.0 - inner_h * peak_n;
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 2.0, top),
                egui::pos2(rect.right() - 2.0, rect.bottom() - 1.0),
            ),
            egui::CornerRadius::same(1),
            col.linear_multiply(0.55),
        );
    }

    if rms_n > 0.005 {
        let top = rect.bottom() - 1.0 - inner_h * rms_n;
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 5.0, top),
                egui::pos2(rect.right() - 5.0, rect.bottom() - 1.0),
            ),
            egui::CornerRadius::same(1),
            MINT,
        );
    }

    if hold_n > 0.01 {
        let y = rect.bottom() - 1.0 - inner_h * hold_n;
        p.line_segment(
            [
                egui::pos2(rect.left() + 2.0, y),
                egui::pos2(rect.right() - 2.0, y),
            ],
            egui::Stroke::new(2.0_f32, INK),
        );
    }
}

fn linear_to_db(v: f32) -> f32 {
    if v <= 1e-5 {
        -60.0
    } else {
        (20.0 * v.log10()).clamp(-60.0, 0.0)
    }
}

fn db_to_meter_norm(db: f32) -> f32 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn format_time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "00:00".to_string();
    }
    let total = seconds.round() as u64;
    let hrs = total / 3600;
    let mins = (total % 3600) / 60;
    let secs = total % 60;
    if hrs > 0 {
        format!("{}:{:02}:{:02}", hrs, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
