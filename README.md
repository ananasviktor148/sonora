# Sonora — Hi-Res Audio Player (Windows & Android)

**Sonora** is a studio-grade high-resolution audio player built in **Rust** for **Windows PC** (`egui`/`eframe`) and **Android** (`Jetpack Compose` + `libsonora_core.so` JNI engine). It combines universal lossless/lossy format playback, native **DSD64–DSD512**, a 64-bit floating-point **Sinc upsampler**, **multi-channel surround (1.0–7.1)**, a **10-band 64-bit parametric EQ**, **KORG AudioGate–style Auto Adjust Playback Volume**, synchronized **Lyrics Studio**, and persistent **Playlists** inside a high-contrast Swiss dark editorial interface.

---

## Features

- **Universal Format Support**
  - **Native Rust PCM (Symphonia)**: FLAC, WAV, ALAC, AIFF, CAF, MP3, AAC, M4A, OGG/Vorbis, MKV/WebM.
  - **Native Rust DSD Engine**: `.dsf` and uncompressed `.dff` up to **DSD512** via a byte-LUT linear-phase Blackman-Harris FIR decimator (`88.2 / 176.4 / 352.8 kHz` 32-bit float PCM) or bit-exact **DoP v1.1** (`0x05/0xFA`) on desktop DACs.
  - **Universal FFmpeg Fallback (Desktop)**: Opus, WavPack (`.wv`), Monkey's Audio (`.ape`), WMA, AC-3, DTS, TrueHD, DST-compressed DFF, and media containers when `ffmpeg` is available on `PATH`.
- **64-Bit Hi-Res Sinc Upsampler (`rubato`)**
  - Bypass (bit-perfect), `2×`, `4×`, `8×` integer-family oversampling, or fixed target sample rates up to `768 kHz` on desktop (`192 kHz` on Android `AudioTrack`), with selectable filter tap length (`64–512` taps), roll-off (`Sharp / Balanced / Gentle`), digital headroom, and soft-clip peak limiting.
- **Auto Adjust Playback Volume (AudioGate Loudness Engine)**
  - Three non-destructive real-time modes: **Off** (master level), **Song** (per-track loudness leveling), and **Album** (preserves track-to-track album dynamics).
  - Full **ITU-R BS.1770-4 / EBU R128** multi-channel K-weighted integrated loudness & peak analyzer, embedded `REPLAYGAIN_*` / `R128_*` tag priority, `-1.0 dBFS` anti-clipping ceiling, smooth click-free `45 ms` gain ramping, persistent disk cache, and automatic DoP bypass.
- **Multi-Channel Surround (1.0 – 7.1)**
  - Native multi-channel passthrough, **ITU-R BS.775** normalized stereo downmix, **Headphone Spatial Crossfeed**, adjustable Center / Surround / LFE gains, per-channel **Mute / Solo**, and live 8-channel Peak / RMS VU meters.
- **10-Band 64-Bit Biquad Equalizer**
  - `31 Hz – 16 kHz` bands, Preamp, and 8 audiophile presets (*Flat, Audiophile Warm, Studio Reference, Vocal Clarity, Deep Bass, Classical Hall, Jazz Club, Electronic*).
- **Synchronized Lyrics Studio**
  - Upload `.lrc`, Enhanced LRC, `.srt`, `.vtt`, or `.txt` (auto UTF-8 / UTF-16 BOM detection), edit inline with a live timestamp stamper, adjust sync offset (`±500 ms`), save sidecar files, embed into audio tags (desktop), or look up synced lyrics via LRCLIB.
- **Persistent Playlists**
  - Create, switch, rename, duplicate, reorder, and delete playlists; right-click / menu to copy tracks across playlists; drag-and-drop and import/export `.m3u` / `.m3u8`.
- **Desktop & Mobile Polish**
  - **Windows PC**: Custom borderless macOS-themed dark title bar with focus-aware traffic lights, 8-edge resize handles, live 48-band FFT spectrum analyzer, Audio Signal Chain inspector, and 5 built-in playable Studio Reference tracks.
  - **Android (`arm64-v8a`, API 26+)**: Native Jetpack Compose Swiss dark UI, low-latency 32-bit float `AudioTrack` foreground playback service, lock-screen / Bluetooth `MediaSession` controls, and Storage Access Framework file import.

---

## Installers & Downloads

Prebuilt installers are published under **GitHub Releases**:
- **Windows (x64)**: `Sonora-Setup-1.0.0-x64.exe` (Inno Setup 6 installer with Start Menu / Desktop shortcuts and optional Hi-Res file associations)
- **Android (arm64-v8a)**: `Sonora-1.0.0-arm64.apk` (Android 8.0+ / API 26+)

---

## Building from Source

### Windows PC (Rust)
```bash
cargo test
cargo build --release
```
To build the Windows installer with **Inno Setup 6**:
```bash
iscc installer\SonoraSetup.iss
```
Output: `dist/Sonora-Setup-1.0.0-x64.exe`.

### Android APK (`arm64-v8a`)
1. Install the Android ARM64 Rust target (`rustup target add aarch64-linux-android`), JDK 17, and Android SDK/NDK.
2. Configure your Android NDK linker in `.cargo/config.toml` and build the native library:
   ```bash
   cargo build --target aarch64-linux-android --lib --release
   ```
3. Copy `target/aarch64-linux-android/release/libsonora_core.so` into `android/app/src/main/jniLibs/arm64-v8a/libsonora_core.so`.
4. Build the APK:
   ```bash
   cd android
   .\gradlew.bat assembleRelease
   ```

---

## Keyboard Shortcuts (Desktop)

| Key | Action |
|---|---|
| `Space` | Play / Pause |
| `Left` / `Right` | Seek `-5s` / `+5s` |
| `Up` / `Down` | Volume `+5%` / `-5%` |
| `M` | Mute / Unmute |
| `N` / `P` | Next / Previous Track |
