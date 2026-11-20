use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem};
use serde::{Deserialize, Serialize};

use crate::audio::demo::demo_spec_for_path;
use crate::audio::dsd::{is_dsd_extension, probe_dsd_file};
use crate::config::AppConfig;

pub const SUPPORTED_LYRICS_EXTENSIONS: &[&str] = &["lrc", "txt", "srt", "vtt"];

pub fn is_lyrics_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let low = ext.to_ascii_lowercase();
            SUPPORTED_LYRICS_EXTENSIONS.iter().any(|&s| s == low)
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedLine {
    pub time_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricsDocument {
    pub raw_text: String,
    pub synced_lines: Vec<SyncedLine>,
    pub plain_lines: Vec<String>,
    pub source: String,
    /// Offset from LRC `[offset:ms]` tag (in milliseconds).
    pub tag_offset_ms: i64,
}

impl LyricsDocument {
    pub fn is_synced(&self) -> bool {
        !self.synced_lines.is_empty()
    }

    /// Find the currently active synced lyric line index for `position_secs`
    /// given user's manual sync adjustment `user_offset_ms`.
    pub fn active_line_index(&self, position_secs: f64, user_offset_ms: i64) -> Option<usize> {
        if self.synced_lines.is_empty() {
            return None;
        }
        let adjusted = position_secs + ((self.tag_offset_ms + user_offset_ms) as f64 / 1000.0);
        if adjusted < self.synced_lines[0].time_secs {
            return None;
        }
        match self
            .synced_lines
            .binary_search_by(|line| line.time_secs.total_cmp(&adjusted))
        {
            Ok(idx) => Some(idx),
            Err(insert_idx) => insert_idx.checked_sub(1),
        }
    }
}

/// Read a lyrics file with automatic UTF-8, UTF-8 BOM, UTF-16LE/BE, and Latin-1 fallback decoding.
pub fn read_text_file_auto_encoding(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(decode_text_bytes(&bytes))
}

pub fn decode_text_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u16s: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let u16s: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_be_bytes(c))
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    // Windows-1252 / Latin-1 lossless fallback
    bytes.iter().map(|&b| b as char).collect()
}

/// Parse LRC, Enhanced LRC, SRT, WebVTT, or Plain Text into a structured `LyricsDocument`.
pub fn parse_lyrics(raw: &str, source: impl Into<String>) -> LyricsDocument {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let source = source.into();

    // Detect SRT / WebVTT subtitle cues (`-->`)
    if normalized.contains("-->") {
        let synced = parse_srt_or_vtt(&normalized);
        if !synced.is_empty() {
            let lrc_raw = synced
                .iter()
                .map(|l| format!("{} {}", format_lrc_timestamp(l.time_secs), l.text))
                .collect::<Vec<_>>()
                .join("\n");
            let plain_lines = synced.iter().map(|l| l.text.clone()).collect();
            return LyricsDocument {
                raw_text: lrc_raw,
                synced_lines: synced,
                plain_lines,
                source,
                tag_offset_ms: 0,
            };
        }
    }

    // Try LRC parsing
    let (synced_lines, tag_offset_ms) = parse_lrc(&normalized);
    let plain_lines: Vec<String> = if !synced_lines.is_empty() {
        synced_lines.iter().map(|l| l.text.clone()).collect()
    } else {
        normalized.lines().map(|l| l.trim_end().to_string()).collect()
    };

    LyricsDocument {
        raw_text: normalized,
        synced_lines,
        plain_lines,
        source,
        tag_offset_ms,
    }
}

fn parse_lrc(text: &str) -> (Vec<SyncedLine>, i64) {
    let mut lines = Vec::new();
    let mut offset_ms = 0i64;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check metadata tags like [offset:+500], [ti:...], [ar:...]
        if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(off_str) = inner.strip_prefix("offset:").or_else(|| inner.strip_prefix("OFFSET:")) {
                if let Ok(ms) = off_str.trim().parse::<i64>() {
                    offset_ms = ms;
                }
                continue;
            }
            // Skip pure metadata header tags like [ti:Song], [ar:Artist], [al:Album], [by:Author]
            if parse_lrc_timestamp_inner(inner).is_none() && inner.contains(':') {
                continue;
            }
        }

        // Extract one or more leading timestamps: [00:12.34][01:20.00] Lyrics text
        let mut rest = trimmed;
        let mut timestamps = Vec::new();
        while let Some(after_open) = rest.strip_prefix('[') {
            if let Some(close_idx) = after_open.find(']') {
                let token = &after_open[..close_idx];
                if let Some(secs) = parse_lrc_timestamp_inner(token) {
                    timestamps.push(secs);
                    rest = &after_open[close_idx + 1..];
                    continue;
                }
            }
            break;
        }

        if !timestamps.is_empty() {
            let cleaned = strip_enhanced_word_tags(rest.trim());
            for t in timestamps {
                lines.push(SyncedLine {
                    time_secs: t,
                    text: cleaned.clone(),
                });
            }
        }
    }

    lines.sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
    (lines, offset_ms)
}

fn parse_lrc_timestamp_inner(s: &str) -> Option<f64> {
    let s = s.trim();
    let (min_str, sec_part) = s.split_once(':')?;
    let mins: f64 = min_str.parse().ok()?;
    if mins < 0.0 {
        return None;
    }
    let secs: f64 = if let Some((whole, frac)) = sec_part.split_once('.').or_else(|| sec_part.split_once(':')) {
        let w: f64 = whole.parse().ok()?;
        let frac_digits = frac.trim();
        let f: f64 = frac_digits.parse().ok()?;
        let denom = 10.0f64.powi(frac_digits.len() as i32);
        w + f / denom
    } else {
        sec_part.parse().ok()?
    };
    Some(mins * 60.0 + secs)
}

/// Strip Enhanced LRC word-level tags `<00:12.34>` so lines display cleanly.
fn strip_enhanced_word_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut closed = false;
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '>' {
                    closed = true;
                    break;
                }
                tag.push(nc);
            }
            if !closed || parse_lrc_timestamp_inner(&tag).is_none() {
                out.push('<');
                out.push_str(&tag);
                if closed {
                    out.push('>');
                }
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

fn parse_srt_or_vtt(text: &str) -> Vec<SyncedLine> {
    let mut result = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();
        if let Some((start_str, _end_str)) = line.split_once("-->") {
            if let Some(start_secs) = parse_subtitle_timestamp(start_str.trim()) {
                i += 1;
                let mut cue_lines = Vec::new();
                while i < lines.len() && !lines[i].trim().is_empty() {
                    let cleaned = strip_html_tags(lines[i].trim());
                    if !cleaned.is_empty() {
                        cue_lines.push(cleaned);
                    }
                    i += 1;
                }
                if !cue_lines.is_empty() {
                    result.push(SyncedLine {
                        time_secs: start_secs,
                        text: cue_lines.join(" "),
                    });
                }
                continue;
            }
        }
        i += 1;
    }

    result.sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
    result
}

fn parse_subtitle_timestamp(s: &str) -> Option<f64> {
    // Handles HH:MM:SS,mmm | HH:MM:SS.mmm | MM:SS.mmm
    let clean = s.split_whitespace().next()?.replace(',', ".");
    let parts: Vec<&str> = clean.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let sec: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + sec)
        }
        _ => None,
    }
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

pub fn format_lrc_timestamp(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let mins = (s / 60.0).floor() as u64;
    let rem = s - (mins as f64) * 60.0;
    format!("[{:02}:{:05.2}]", mins, rem)
}

// ── Persistent User Lyrics Store & Upload / Save / Embed ─────────────────────

fn track_lyrics_store_path(track_path: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    track_path.to_string_lossy().to_lowercase().hash(&mut hasher);
    let hash = hasher.finish();
    let stem = track_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("track")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect::<String>();
    AppConfig::lyrics_store_dir().join(format!("{}_{:016x}.lrc", stem, hash))
}

/// Upload a lyrics file (`.lrc`, `.txt`, `.srt`, `.vtt`) for `track_path`,
/// parse it, and persist it in the app's lyrics store so it reloads automatically.
pub fn upload_lyrics_for_track(track_path: &Path, lyrics_file: &Path) -> Result<LyricsDocument> {
    let raw = read_text_file_auto_encoding(lyrics_file)?;
    let fname = lyrics_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("uploaded");
    let doc = parse_lyrics(&raw, format!("Uploaded ({})", fname));
    save_to_lyrics_store(track_path, &doc.raw_text)?;
    Ok(doc)
}

/// Persist edited/uploaded lyrics text in the app's persistent lyrics store.
pub fn save_to_lyrics_store(track_path: &Path, text: &str) -> Result<PathBuf> {
    let dest = track_lyrics_store_path(track_path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&dest, text)?;
    Ok(dest)
}

/// Save lyrics as a sidecar `.lrc` (if synced) or `.txt` file right next to the audio file.
pub fn save_sidecar_lyrics(track_path: &Path, doc: &LyricsDocument) -> Result<PathBuf> {
    let ext = if doc.is_synced() { "lrc" } else { "txt" };
    let sidecar = track_path.with_extension(ext);
    fs::write(&sidecar, &doc.raw_text)?;
    Ok(sidecar)
}

/// Embed lyrics directly into the audio file's metadata tags using `lofty`.
pub fn embed_lyrics_in_audio_file(track_path: &Path, text: &str) -> Result<()> {
    let mut tagged = lofty::read_from_path(track_path)
        .map_err(|e| anyhow!("Cannot open tags for embedding: {}", e))?;

    let tag = match tagged.primary_tag_mut() {
        Some(t) => t,
        None => {
            if let Some(t) = tagged.first_tag_mut() {
                t
            } else {
                let tag_type = tagged.primary_tag_type();
                tagged.insert_tag(Tag::new(tag_type));
                tagged
                    .primary_tag_mut()
                    .ok_or_else(|| anyhow!("Failed to create tag for this format"))?
            }
        }
    };

    tag.remove_key(&ItemKey::Lyrics);
    tag.insert(TagItem::new(
        ItemKey::Lyrics,
        ItemValue::Text(text.to_string()),
    ));
    tagged
        .save_to_path(track_path, WriteOptions::default())
        .map_err(|e| anyhow!("Failed to write embedded lyrics: {}", e))?;
    Ok(())
}

/// Auto-discover local lyrics for a track:
/// 1. User-uploaded / saved lyrics in app store
/// 2. Sidecar `.lrc`, `.srt`, `.vtt`, `.txt` next to the audio file
/// 3. Embedded metadata lyrics (lofty + DSF/DFF ID3v2)
pub fn load_local_lyrics(track_path: &Path) -> Option<LyricsDocument> {
    // 1. User-uploaded / saved store
    let store_path = track_lyrics_store_path(track_path);
    if store_path.exists() {
        if let Ok(raw) = read_text_file_auto_encoding(&store_path) {
            if !raw.trim().is_empty() {
                return Some(parse_lyrics(&raw, "Saved / Uploaded"));
            }
        }
    }

    // 2. Sidecar files next to audio track
    for ext in SUPPORTED_LYRICS_EXTENSIONS {
        let sidecar = track_path.with_extension(ext);
        if sidecar.exists() {
            if let Ok(raw) = read_text_file_auto_encoding(&sidecar) {
                if !raw.trim().is_empty() {
                    let fname = sidecar
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("sidecar");
                    return Some(parse_lyrics(&raw, format!("Sidecar ({})", fname)));
                }
            }
        }
    }

    // 3. Embedded lyrics in audio tags (DSF/DFF ID3v2 or lofty)
    if let Some(embedded) = read_embedded_lyrics(track_path) {
        return Some(parse_lyrics(&embedded, "Embedded Tag"));
    }

    // 4. Built-in studio reference showcase synced lyrics
    if let Some(spec) = demo_spec_for_path(track_path) {
        return Some(parse_lyrics(
            spec.synced_lyrics,
            format!("Studio Sheet · {}", spec.catalog_no),
        ));
    }

    None
}

pub fn read_embedded_lyrics(path: &Path) -> Option<String> {
    if is_dsd_extension(path) {
        if let Ok(dsd) = probe_dsd_file(path) {
            if let Some(l) = dsd.metadata.lyrics {
                if !l.trim().is_empty() {
                    return Some(l);
                }
            }
        }
    }

    let tagged = lofty::read_from_path(path).ok()?;
    for tag in tagged.tags() {
        if let Some(text) = tag.get_string(&ItemKey::Lyrics) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
        for item in tag.items() {
            let key_low = format!("{:?}", item.key()).to_ascii_lowercase();
            if key_low.contains("lyric") || key_low.contains("uslt") || key_low.contains("sylt") {
                if let ItemValue::Text(t) = item.value() {
                    if !t.trim().is_empty() {
                        return Some(t.clone());
                    }
                }
            }
        }
    }
    None
}

// ── Optional Online Lyrics Lookup (LRCLIB) ───────────────────────────────────

#[cfg(not(target_os = "android"))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibItem {
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

#[cfg(not(target_os = "android"))]
pub fn fetch_online_lyrics(artist: &str, title: &str) -> Option<LyricsDocument> {
    let clean_title = title.trim();
    let clean_artist = artist.trim();
    if clean_title.is_empty() && clean_artist.is_empty() {
        return None;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .user_agent("HiResAudioPlayer/1.0")
        .build()
        .ok()?;

    // 1. Exact lookup
    let get_url = format!(
        "https://lrclib.net/api/get?artist_name={}&track_name={}",
        urlencode(clean_artist),
        urlencode(clean_title)
    );
    if let Ok(resp) = client.get(&get_url).send() {
        if resp.status().is_success() {
            if let Ok(item) = resp.json::<LrclibItem>() {
                if let Some(doc) = item_to_doc(item) {
                    return Some(doc);
                }
            }
        }
    }

    // 2. Fallback search query
    let q = if clean_artist.is_empty() {
        clean_title.to_string()
    } else {
        format!("{} {}", clean_artist, clean_title)
    };
    let search_url = format!("https://lrclib.net/api/search?q={}", urlencode(&q));
    if let Ok(resp) = client.get(&search_url).send() {
        if resp.status().is_success() {
            if let Ok(items) = resp.json::<Vec<LrclibItem>>() {
                for item in items {
                    if let Some(doc) = item_to_doc(item) {
                        return Some(doc);
                    }
                }
            }
        }
    }

    None
}

#[cfg(not(target_os = "android"))]
fn item_to_doc(item: LrclibItem) -> Option<LyricsDocument> {
    if let Some(synced) = item.synced_lyrics {
        if !synced.trim().is_empty() {
            return Some(parse_lyrics(&synced, "LRCLIB (Synced)"));
        }
    }
    if let Some(plain) = item.plain_lyrics {
        if !plain.trim().is_empty() {
            return Some(parse_lyrics(&plain, "LRCLIB (Plain)"));
        }
    }
    None
}

#[cfg(not(target_os = "android"))]
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push('+'),
            _ => {
                for b in c.to_string().as_bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_with_multi_timestamps_offset_and_enhanced_tags() {
        let lrc = "\
[ti:Test Song]
[ar:Audiophile]
[offset:500]
[00:05.00] First <00:05.50>line here
[00:10.25][00:30.50] Chorus line
[00:20.00] Verse two
";
        let doc = parse_lyrics(lrc, "test.lrc");
        assert!(doc.is_synced());
        assert_eq!(doc.tag_offset_ms, 500);
        assert_eq!(doc.synced_lines.len(), 4);
        assert_eq!(doc.synced_lines[0].text, "First line here");
        assert!((doc.synced_lines[1].time_secs - 10.25).abs() < 1e-4);
        assert_eq!(doc.synced_lines[1].text, "Chorus line");
        assert_eq!(doc.synced_lines[2].text, "Verse two");
        assert!((doc.synced_lines[3].time_secs - 30.50).abs() < 1e-4);

        // At 10.0s with +500ms offset -> effective 10.5s -> line 1 ("Chorus line")
        assert_eq!(doc.active_line_index(10.0, 0), Some(1));
        // With -1000ms user nudge -> effective 9.5s -> line 0 ("First line here")
        assert_eq!(doc.active_line_index(10.0, -1000), Some(0));
    }

    #[test]
    fn parses_srt_and_vtt_subtitles_into_synced_lyrics() {
        let srt = "\
1
00:00:02,500 --> 00:00:05,000
<i>Hello world</i>
Second line

2
00:01:05,250 --> 00:01:08,000
Next verse
";
        let doc = parse_lyrics(srt, "song.srt");
        assert!(doc.is_synced());
        assert_eq!(doc.synced_lines.len(), 2);
        assert!((doc.synced_lines[0].time_secs - 2.5).abs() < 1e-4);
        assert_eq!(doc.synced_lines[0].text, "Hello world Second line");
        assert!((doc.synced_lines[1].time_secs - 65.25).abs() < 1e-4);
    }

    #[test]
    fn decodes_utf16le_with_bom() {
        let mut bytes = vec![0xFFu8, 0xFE];
        for cu in "[00:01.00] Hi-Res".encode_utf16() {
            bytes.extend_from_slice(&cu.to_le_bytes());
        }
        let decoded = decode_text_bytes(&bytes);
        assert_eq!(decoded, "[00:01.00] Hi-Res");
    }

    #[test]
    fn upload_lyrics_persists_and_auto_reloads() {
        let tmp = std::env::temp_dir();
        let fake_track = tmp.join("sonora_unit_test_unique_track_99182.flac");
        let lrc_file = tmp.join("sonora_uploaded_lyrics_99182.lrc");
        fs::write(&lrc_file, "[00:01.00] Line One\n[00:04.50] Line Two\n").unwrap();

        let uploaded = upload_lyrics_for_track(&fake_track, &lrc_file).unwrap();
        assert!(uploaded.is_synced());
        assert_eq!(uploaded.synced_lines.len(), 2);

        // Remove original uploaded file; track should still auto-reload from persistent store
        let _ = fs::remove_file(&lrc_file);
        let reloaded = load_local_lyrics(&fake_track).expect("should reload persisted lyrics");
        assert_eq!(reloaded.synced_lines.len(), 2);
        assert_eq!(reloaded.synced_lines[1].text, "Line Two");

        // Clean up stored test artifact
        let store_path = track_lyrics_store_path(&fake_track);
        let _ = fs::remove_file(&store_path);
    }
}
