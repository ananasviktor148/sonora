use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::audio::decoder::{is_supported_audio_file, probe_track_info, TrackInfo};
use crate::audio::demo::{demo_spec_for_path, demo_track_info, DEMO_CATALOG};
use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    pub duration: f64,
    pub sample_rate: u32,
    pub bit_depth: u16,
    pub channels: u16,
    pub format: String,
    pub codec: String,
    pub is_dsd: bool,
    pub is_hires: bool,
    #[serde(default)]
    pub is_demo: bool,
}

impl PlaylistEntry {
    pub fn from_info(info: TrackInfo) -> Self {
        let is_hires = info.is_hires();
        let is_demo = demo_spec_for_path(&info.file_path).is_some();
        Self {
            path: info.file_path,
            title: info.title,
            artist: info.artist,
            album: info.album,
            duration: info.duration,
            sample_rate: info.sample_rate,
            bit_depth: info.bit_depth,
            channels: info.channels,
            format: info.format,
            codec: info.codec,
            is_dsd: info.is_dsd,
            is_hires,
            is_demo,
        }
    }

    pub fn spec_code(&self) -> String {
        if self.is_dsd {
            if self.channels > 2 {
                format!("{} · {}CH", self.codec, self.channels)
            } else {
                self.codec.clone()
            }
        } else if self.sample_rate > 0 {
            let sr = if self.sample_rate >= 1000 {
                let k = self.sample_rate as f64 / 1000.0;
                if (k - k.round()).abs() < 0.05 {
                    format!("{}K", k.round() as u32)
                } else {
                    format!("{:.1}K", k)
                }
            } else {
                format!("{}HZ", self.sample_rate)
            };
            if self.channels > 2 {
                format!(
                    "{} {}/{} · {}CH",
                    self.format, sr, self.bit_depth, self.channels
                )
            } else {
                format!("{} {}/{}", self.format, sr, self.bit_depth)
            }
        } else {
            self.format.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<PlaylistEntry>,
}

impl Playlist {
    pub fn new(id: impl Into<String>, name: impl Into<String>, tracks: Vec<PlaylistEntry>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            tracks,
        }
    }

    pub fn total_duration(&self) -> f64 {
        self.tracks.iter().map(|t| t.duration).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistLibrary {
    pub playlists: Vec<Playlist>,
    pub active_id: String,
    #[serde(default)]
    next_seq: u64,
}

impl PlaylistLibrary {
    pub fn default_with_references() -> Self {
        let demo_tracks: Vec<PlaylistEntry> = DEMO_CATALOG
            .iter()
            .map(|spec| PlaylistEntry::from_info(demo_track_info(spec)))
            .collect();
        let id = "pl-studio-reference".to_string();
        Self {
            playlists: vec![Playlist::new(id.clone(), "Studio Reference", demo_tracks)],
            active_id: id,
            next_seq: 1,
        }
    }

    pub fn load() -> Self {
        let path = AppConfig::playlists_path();
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(mut lib) = serde_json::from_str::<Self>(&raw) {
                if !lib.playlists.is_empty() {
                    if !lib.playlists.iter().any(|p| p.id == lib.active_id) {
                        lib.active_id = lib.playlists[0].id.clone();
                    }
                    return lib;
                }
            }
        }
        Self::default_with_references()
    }

    pub fn save(&self) {
        let path = AppConfig::playlists_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    fn alloc_id(&mut self, name: &str) -> String {
        self.next_seq += 1;
        let slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .take(24)
            .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("pl-{}-{}-{}", self.next_seq, slug.trim_matches('-'), now % 100_000)
    }

    pub fn active(&self) -> Option<&Playlist> {
        self.playlists
            .iter()
            .find(|p| p.id == self.active_id)
            .or_else(|| self.playlists.first())
    }

    pub fn active_mut(&mut self) -> Option<&mut Playlist> {
        let active_id = self.active_id.clone();
        if let Some(idx) = self.playlists.iter().position(|p| p.id == active_id) {
            return self.playlists.get_mut(idx);
        }
        self.playlists.first_mut()
    }

    pub fn get(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    /// Create a new playlist (optionally seeded with tracks), make it active, and return its ID.
    pub fn create_playlist(&mut self, name: &str, tracks: Vec<PlaylistEntry>) -> String {
        let clean = name.trim();
        let final_name = if clean.is_empty() {
            format!("Playlist {:02}", self.playlists.len() + 1)
        } else {
            clean.to_string()
        };
        let id = self.alloc_id(&final_name);
        self.playlists
            .push(Playlist::new(id.clone(), final_name, tracks));
        self.active_id = id.clone();
        id
    }

    pub fn rename_playlist(&mut self, id: &str, new_name: &str) -> bool {
        let clean = new_name.trim();
        if clean.is_empty() {
            return false;
        }
        if let Some(pl) = self.get_mut(id) {
            pl.name = clean.to_string();
            return true;
        }
        false
    }

    pub fn duplicate_playlist(&mut self, id: &str) -> Option<String> {
        let (name, tracks) = {
            let pl = self.get(id)?;
            (format!("{} (Copy)", pl.name), pl.tracks.clone())
        };
        Some(self.create_playlist(&name, tracks))
    }

    /// Delete a playlist by ID. Always preserves at least one playlist (creates an empty "Queue" if deleting the last one).
    pub fn delete_playlist(&mut self, id: &str) -> bool {
        let Some(pos) = self.playlists.iter().position(|p| p.id == id) else {
            return false;
        };
        self.playlists.remove(pos);
        if self.playlists.is_empty() {
            let new_id = self.alloc_id("queue");
            self.playlists
                .push(Playlist::new(new_id.clone(), "Queue", Vec::new()));
            self.active_id = new_id;
        } else if self.active_id == id {
            let next_idx = pos.min(self.playlists.len() - 1);
            self.active_id = self.playlists[next_idx].id.clone();
        }
        true
    }

    pub fn add_track_to(&mut self, playlist_id: &str, entry: PlaylistEntry) -> bool {
        if let Some(pl) = self.get_mut(playlist_id) {
            pl.tracks.push(entry);
            return true;
        }
        false
    }
}

// ── M3U / M3U8 Playlist Import & Export ──────────────────────────────────────

pub fn is_playlist_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("m3u") || ext.eq_ignore_ascii_case("m3u8"))
        .unwrap_or(false)
}

/// Import a standard `.m3u` or `.m3u8` playlist file into a list of `PlaylistEntry` items.
pub fn parse_m3u_file(path: &Path) -> Result<(String, Vec<PlaylistEntry>)> {
    let raw = fs::read_to_string(path)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut playlist_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Playlist")
        .to_string();

    let mut entries = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() {
            continue;
        }
        if let Some(pl_title) = trimmed.strip_prefix("#PLAYLIST:") {
            let clean = pl_title.trim();
            if !clean.is_empty() {
                playlist_name = clean.to_string();
            }
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        let candidate = PathBuf::from(trimmed);
        let resolved = if candidate.is_absolute() {
            candidate
        } else {
            base_dir.join(candidate)
        };
        if resolved.exists() && is_supported_audio_file(&resolved) {
            entries.push(PlaylistEntry::from_info(probe_track_info(&resolved)));
        }
    }

    Ok((playlist_name, entries))
}

/// Export a playlist to UTF-8 `.m3u8` format.
pub fn export_m3u8(playlist: &Playlist, dest: &Path) -> Result<()> {
    let mut out = String::from("#EXTM3U\n");
    out.push_str(&format!("#PLAYLIST:{}\n", playlist.name));
    for track in &playlist.tracks {
        if track.is_demo {
            continue;
        }
        let secs = track.duration.round() as i64;
        let label = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{} - {}", track.artist, track.title)
        };
        out.push_str(&format!("#EXTINF:{},{}\n", secs, label));
        out.push_str(&track.path.to_string_lossy());
        out.push('\n');
    }
    fs::write(dest, out)?;
    Ok(())
}

// ── Fisher-Yates Non-Repeating Shuffle Deck ──────────────────────────────────

#[derive(Debug, Clone)]
pub struct ShuffleDeck {
    playlist_id: String,
    playlist_len: usize,
    order: Vec<usize>,
    cursor: usize,
    rng_state: u64,
}

impl Default for ShuffleDeck {
    fn default() -> Self {
        Self::new()
    }
}

impl ShuffleDeck {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self {
            playlist_id: String::new(),
            playlist_len: 0,
            order: Vec::new(),
            cursor: 0,
            rng_state: seed ^ 0xA5A5_5A5A_F0F0_0F0F,
        }
    }

    pub fn reset(&mut self) {
        self.order.clear();
        self.playlist_len = 0;
        self.cursor = 0;
    }

    pub fn on_tracks_swapped(&mut self, playlist_id: &str, a: usize, b: usize) {
        if self.playlist_id != playlist_id || a == b {
            return;
        }
        for item in &mut self.order {
            if *item == a {
                *item = b;
            } else if *item == b {
                *item = a;
            }
        }
    }

    /// SplitMix64 high-quality 64-bit PRNG (passes BigCrush, zero modulo bias on small ranges).
    fn next_u64(&mut self) -> u64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        self.rng_state = self
            .rng_state
            .wrapping_add(0x9E37_79B9_7F4A_7C15 ^ nanos.rotate_left(17));
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn random_below(&mut self, bound: usize) -> usize {
        if bound <= 1 {
            return 0;
        }
        (self.next_u64() % (bound as u64)) as usize
    }

    fn build_cycle(&mut self, playlist_id: &str, len: usize, anchor_first: Option<usize>, avoid_first: Option<usize>) {
        self.playlist_id = playlist_id.to_string();
        self.playlist_len = len;
        self.order = (0..len).collect();
        self.cursor = 0;

        if len <= 1 {
            return;
        }

        // Unbiased Fisher-Yates shuffle
        for i in (1..len).rev() {
            let j = self.random_below(i + 1);
            self.order.swap(i, j);
        }

        if let Some(first) = anchor_first.filter(|&idx| idx < len) {
            if let Some(pos) = self.order.iter().position(|&x| x == first) {
                self.order.swap(0, pos);
            }
            // Re-randomize tail 1..len so the remaining N-1 tracks are uniformly permuted
            for i in (2..len).rev() {
                let j = 1 + self.random_below(i);
                self.order.swap(i, j);
            }
        } else if let Some(avoid) = avoid_first.filter(|&idx| idx < len) {
            if self.order[0] == avoid {
                let swap_with = 1 + self.random_below(len - 1);
                self.order.swap(0, swap_with);
            }
        }
    }

    /// Synchronize deck when a track is loaded. If `idx` matches the current shuffle cursor,
    /// preserves the cycle; if the user manually picked a new track, starts a fresh cycle from `idx`.
    pub fn on_track_loaded(&mut self, playlist_id: &str, len: usize, idx: usize) {
        if len == 0 || idx >= len {
            self.reset();
            return;
        }
        if self.playlist_id == playlist_id
            && self.playlist_len == len
            && self.order.len() == len
            && self.order.get(self.cursor) == Some(&idx)
        {
            return;
        }
        self.build_cycle(playlist_id, len, Some(idx), None);
    }

    /// Return the next track index in the non-repeating shuffle bag.
    /// Every track in the playlist is visited once before any track repeats.
    pub fn next_index(
        &mut self,
        playlist_id: &str,
        len: usize,
        current_index: Option<usize>,
        allow_wrap: bool,
    ) -> Option<usize> {
        if len == 0 {
            return None;
        }
        if len == 1 {
            return if allow_wrap || current_index.is_none() {
                Some(0)
            } else {
                None
            };
        }

        let valid_deck = self.playlist_id == playlist_id
            && self.playlist_len == len
            && self.order.len() == len;

        if !valid_deck {
            if let Some(cur) = current_index.filter(|&i| i < len) {
                self.build_cycle(playlist_id, len, Some(cur), None);
                self.cursor = 1;
                return Some(self.order[1]);
            } else {
                self.build_cycle(playlist_id, len, None, None);
                self.cursor = 0;
                return Some(self.order[0]);
            }
        }

        // If current_index drifted from cursor, resync to its position in the current bag
        if let Some(cur) = current_index.filter(|&i| i < len) {
            if self.order.get(self.cursor) != Some(&cur) {
                if let Some(pos) = self.order.iter().position(|&x| x == cur) {
                    self.cursor = pos;
                }
            }
        }

        if self.cursor + 1 < len {
            self.cursor += 1;
            Some(self.order[self.cursor])
        } else if allow_wrap {
            let last_played = self.order.last().copied().or(current_index);
            self.build_cycle(playlist_id, len, None, last_played);
            self.cursor = 0;
            Some(self.order[0])
        } else {
            None
        }
    }

    /// Step backward through the shuffled play order.
    pub fn prev_index(
        &mut self,
        playlist_id: &str,
        len: usize,
        current_index: Option<usize>,
    ) -> Option<usize> {
        if len == 0 {
            return None;
        }
        if len == 1 {
            return Some(0);
        }

        let valid_deck = self.playlist_id == playlist_id
            && self.playlist_len == len
            && self.order.len() == len;

        if !valid_deck {
            self.build_cycle(playlist_id, len, current_index, None);
            self.cursor = len - 1;
            return Some(self.order[self.cursor]);
        }

        if self.cursor > 0 {
            self.cursor -= 1;
        } else {
            self.cursor = len - 1;
        }
        Some(self.order[self.cursor])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_rename_duplicate_add_and_delete_playlists() {
        let mut lib = PlaylistLibrary::default_with_references();
        assert_eq!(lib.playlists.len(), 1);
        let ref_id = lib.active_id.clone();
        let sample_track = lib.active().unwrap().tracks[0].clone();

        // Create a new custom playlist
        let jazz_id = lib.create_playlist("Late Night Jazz", Vec::new());
        assert_eq!(lib.playlists.len(), 2);
        assert_eq!(lib.active_id, jazz_id);
        assert_eq!(lib.active().unwrap().name, "Late Night Jazz");
        assert!(lib.active().unwrap().tracks.is_empty());

        // Add track to it
        assert!(lib.add_track_to(&jazz_id, sample_track));
        assert_eq!(lib.get(&jazz_id).unwrap().tracks.len(), 1);

        // Rename it
        assert!(lib.rename_playlist(&jazz_id, "Stockholm Jazz 192k"));
        assert_eq!(lib.get(&jazz_id).unwrap().name, "Stockholm Jazz 192k");

        // Duplicate it
        let dup_id = lib.duplicate_playlist(&jazz_id).unwrap();
        assert_eq!(lib.playlists.len(), 3);
        assert_eq!(lib.get(&dup_id).unwrap().name, "Stockholm Jazz 192k (Copy)");
        assert_eq!(lib.get(&dup_id).unwrap().tracks.len(), 1);

        // Delete playlists cleanly (always leaves at least 1)
        assert!(lib.delete_playlist(&dup_id));
        assert!(lib.delete_playlist(&jazz_id));
        assert!(lib.delete_playlist(&ref_id));
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(lib.active().unwrap().name, "Queue");
    }

    #[test]
    fn m3u8_export_and_import_round_trip() {
        let tmp = std::env::temp_dir();
        let dummy_wav = tmp.join("sonora_m3u_test_track.wav");
        // Write minimal valid 16-bit WAV header so probe_track_info succeeds
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&(44100u32 * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&0u32.to_le_bytes());
        fs::write(&dummy_wav, wav).unwrap();

        let entry = PlaylistEntry::from_info(probe_track_info(&dummy_wav));
        let pl = Playlist::new("pl-test", "Audiophile Mix", vec![entry]);
        let m3u_path = tmp.join("Audiophile Mix.m3u8");
        export_m3u8(&pl, &m3u_path).unwrap();

        let (name, imported) = parse_m3u_file(&m3u_path).unwrap();
        assert_eq!(name, "Audiophile Mix");
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].path, dummy_wav);

        let _ = fs::remove_file(&m3u_path);
        let _ = fs::remove_file(&dummy_wav);
    }

    #[test]
    fn shuffle_deck_plays_every_track_once_per_cycle_without_repeats() {
        let mut deck = ShuffleDeck::new();
        let pid = "pl-test";
        let len = 8usize;

        // Start playing track 3
        deck.on_track_loaded(pid, len, 3);
        let mut seen = vec![3usize];
        for _ in 1..len {
            let next = deck
                .next_index(pid, len, seen.last().copied(), false)
                .expect("should yield remaining tracks in cycle");
            assert!(
                !seen.contains(&next),
                "track {} repeated before cycle finished (seen={:?})",
                next,
                seen
            );
            seen.push(next);
        }
        assert_eq!(seen.len(), len);

        // With allow_wrap = false, cycle end returns None
        assert_eq!(deck.next_index(pid, len, seen.last().copied(), false), None);

        // With allow_wrap = true, starts a new cycle and never repeats the last track back-to-back
        let last_of_cycle1 = *seen.last().unwrap();
        let first_of_cycle2 = deck
            .next_index(pid, len, Some(last_of_cycle1), true)
            .expect("should wrap into new shuffle cycle");
        assert_ne!(
            first_of_cycle2, last_of_cycle1,
            "must not repeat last track of cycle 1 at start of cycle 2"
        );

        let mut cycle2 = vec![first_of_cycle2];
        for _ in 1..len {
            let next = deck
                .next_index(pid, len, cycle2.last().copied(), false)
                .unwrap();
            assert!(!cycle2.contains(&next));
            cycle2.push(next);
        }
        assert_eq!(cycle2.len(), len);
    }
}
