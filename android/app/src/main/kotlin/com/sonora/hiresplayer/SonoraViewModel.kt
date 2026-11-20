package com.sonora.hiresplayer

import android.app.Application
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.MessageDigest

class SonoraViewModel(application: Application) : AndroidViewModel(application) {

  private val appContext = application.applicationContext

  private val _state = MutableStateFlow(SonoraState())
  val state: StateFlow<SonoraState> = _state.asStateFlow()

  private val _visualizer = MutableStateFlow(FloatArray(120))
  val visualizer: StateFlow<FloatArray> = _visualizer.asStateFlow()

  private val _coverBitmap = MutableStateFlow<ImageBitmap?>(null)
  val coverBitmap: StateFlow<ImageBitmap?> = _coverBitmap.asStateFlow()

  private val _toastMessage = MutableStateFlow<String?>(null)
  val toastMessage: StateFlow<String?> = _toastMessage.asStateFlow()

  private val _fetchingLyrics = MutableStateFlow(false)
  val fetchingLyrics: StateFlow<Boolean> = _fetchingLyrics.asStateFlow()

  private var lastCoverRevision: Long = -1L

  init {
    SonoraNative.nativeInit(appContext.filesDir.absolutePath)
    SonoraPlaybackService.sendAction(appContext, SonoraPlaybackService.ACTION_START)
    refreshNow()
    startPollingLoops()
  }

  private fun startPollingLoops() {
    // State snapshot loop (~5 Hz playing, ~2 Hz idle)
    viewModelScope.launch(Dispatchers.Default) {
      while (true) {
        refreshSnapshotInternal()
        delay(if (_state.value.isPlaying) 180L else 450L)
      }
    }
    // Visualizer loop (~30 fps playing, slow decay idle)
    viewModelScope.launch(Dispatchers.Default) {
      while (true) {
        val floats = SonoraNative.nativeGetVisualizerFloats()
        if (floats.size >= 120) {
          _visualizer.value = floats
        }
        val active = _state.value.isPlaying || floats.any { it > 0.005f }
        delay(if (active) 33L else 180L)
      }
    }
  }

  fun showToast(msg: String) {
    _toastMessage.value = msg
    viewModelScope.launch {
      delay(3200L)
      if (_toastMessage.value == msg) _toastMessage.value = null
    }
  }

  fun refreshNow() {
    viewModelScope.launch(Dispatchers.Default) {
      refreshSnapshotInternal()
    }
  }

  private fun refreshSnapshotInternal() {
    val snap = SonoraJsonParser.parseSnapshot(SonoraNative.nativeGetStateJson())
    _state.value = snap
    snap.toast?.let { showToast(it) }
    if (snap.coverRevision != lastCoverRevision) {
      lastCoverRevision = snap.coverRevision
      val raw = SonoraNative.nativeGetCoverArtBytes()
      _coverBitmap.value = if (raw.isNotEmpty()) {
        BitmapFactory.decodeByteArray(raw, 0, raw.size)?.asImageBitmap()
      } else null
    }
  }

  private fun runEngineAction(refreshService: Boolean = false, block: () -> Unit) {
    viewModelScope.launch(Dispatchers.Default) {
      block()
      refreshSnapshotInternal()
      if (refreshService) {
        SonoraPlaybackService.sendAction(appContext, SonoraPlaybackService.ACTION_REFRESH)
      }
    }
  }

  // ── Transport & Playback ───────────────────────────────────────────────────

  fun togglePlayPause() = runEngineAction(refreshService = true) { SonoraNative.nativeTogglePlayPause() }
  fun stop() = runEngineAction(refreshService = true) { SonoraNative.nativeStop() }
  fun nextTrack() = runEngineAction(refreshService = true) { SonoraNative.nativeNext(true) }
  fun prevTrack() = runEngineAction(refreshService = true) { SonoraNative.nativePrev() }
  fun seekTo(seconds: Double) = runEngineAction(refreshService = true) { SonoraNative.nativeSeek(seconds) }
  fun loadTrack(playlistId: String, index: Int, autoplay: Boolean = true) =
    runEngineAction(refreshService = true) { SonoraNative.nativeLoadTrack(playlistId, index, autoplay) }

  fun setVolume(vol: Float) = runEngineAction { SonoraNative.nativeSetVolume(vol) }
  fun toggleMute() = runEngineAction { SonoraNative.nativeSetMuted(!_state.value.muted) }
  fun toggleShuffle() = runEngineAction { SonoraNative.nativeToggleShuffle() }
  fun cycleRepeat() = runEngineAction { SonoraNative.nativeCycleRepeat() }

  // ── DSP / Upsampler / DSD / Multi-Channel / EQ / AudioGate Auto Volume ────

  fun updateConfig(transform: (ConfigUi) -> ConfigUi) {
    val updated = transform(_state.value.config)
    runEngineAction(refreshService = true) {
      SonoraNative.nativeUpdateConfigJson(updated.toRustJson())
    }
  }

  fun cycleAutoVolume() = runEngineAction { SonoraNative.nativeCycleAutoVolume() }
  fun reanalyzeAutoVolume() = runEngineAction { SonoraNative.nativeReanalyzeAutoVolume() }

  fun toggleChannelMute(ch: Int) {
    val s = _state.value
    val curM = s.channelMuted.getOrElse(ch) { false }
    val curS = s.channelSolo.getOrElse(ch) { false }
    runEngineAction { SonoraNative.nativeSetChannelMuteSolo(ch, !curM, curS) }
  }

  fun toggleChannelSolo(ch: Int) {
    val s = _state.value
    val curM = s.channelMuted.getOrElse(ch) { false }
    val curS = s.channelSolo.getOrElse(ch) { false }
    runEngineAction { SonoraNative.nativeSetChannelMuteSolo(ch, curM, !curS) }
  }

  fun resetMuteSolo() = runEngineAction { SonoraNative.nativeResetMuteSolo() }

  // ── Playlists ──────────────────────────────────────────────────────────────

  fun createPlaylist(name: String, copyCurrent: Boolean, seedIndex: Int = -1) =
    runEngineAction { SonoraNative.nativeCreatePlaylist(name, copyCurrent, seedIndex) }

  fun switchPlaylist(id: String) = runEngineAction { SonoraNative.nativeSwitchPlaylist(id) }
  fun renamePlaylist(id: String, newName: String) = runEngineAction { SonoraNative.nativeRenamePlaylist(id, newName) }
  fun duplicatePlaylist(id: String) = runEngineAction { SonoraNative.nativeDuplicatePlaylist(id) }
  fun deletePlaylist(id: String) = runEngineAction { SonoraNative.nativeDeletePlaylist(id) }
  fun addTrackToPlaylist(targetId: String, sourceIndex: Int) =
    runEngineAction { SonoraNative.nativeAddTrackToPlaylist(targetId, sourceIndex) }
  fun removeTrack(index: Int) = runEngineAction { SonoraNative.nativeRemoveTrack(index) }
  fun moveTrack(from: Int, to: Int) = runEngineAction { SonoraNative.nativeMoveTrack(from, to) }
  fun clearActivePlaylist() = runEngineAction(refreshService = true) { SonoraNative.nativeClearActivePlaylist() }
  fun restoreStudioReferences() = runEngineAction(refreshService = true) { SonoraNative.nativeRestoreStudioReferences() }

  // ── SAF File Import (Audio / Playlists / Lyrics) ───────────────────────────

  fun importAudioUris(uris: List<Uri>) {
    if (uris.isEmpty()) return
    viewModelScope.launch(Dispatchers.IO) {
      val importDir = File(appContext.filesDir, "imported_audio").apply { mkdirs() }
      val importedPaths = mutableListOf<String>()
      for (uri in uris) {
        val displayName = resolveDisplayName(uri) ?: "track_${System.currentTimeMillis()}.flac"
        val safeName = displayName.replace(Regex("[^A-Za-z0-9._ -]"), "_")
        val prefix = shortHash(uri.toString())
        val destFile = File(importDir, "${prefix}_$safeName")
        if (!destFile.exists() || destFile.length() == 0L) {
          try {
            appContext.contentResolver.openInputStream(uri)?.use { input ->
              destFile.outputStream().use { output -> input.copyTo(output) }
            }
          } catch (_: Exception) {
            continue
          }
        }
        if (destFile.exists() && destFile.length() > 0L) {
          importedPaths.add(destFile.absolutePath)
        }
      }
      if (importedPaths.isNotEmpty()) {
        val arr = JSONArray()
        importedPaths.forEach { arr.put(it) }
        SonoraNative.nativeImportPathsJson(arr.toString())
        refreshSnapshotInternal()
        SonoraPlaybackService.sendAction(appContext, SonoraPlaybackService.ACTION_REFRESH)
      } else {
        showToast("NO SUPPORTED AUDIO FILES IMPORTED")
      }
    }
  }

  fun importLyricsUri(uri: Uri) {
    viewModelScope.launch(Dispatchers.IO) {
      try {
        val name = resolveDisplayName(uri) ?: "uploaded.lrc"
        val bytes = appContext.contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: return@launch
        val text = decodeTextAuto(bytes)
        SonoraNative.nativeUploadLyricsText(text, "Uploaded · $name")
        refreshSnapshotInternal()
      } catch (e: Exception) {
        showToast("LYRICS UPLOAD FAILED · ${e.message}")
      }
    }
  }

  fun saveLyricsText(rawText: String, sourceLabel: String = "Custom Transcript") =
    runEngineAction { SonoraNative.nativeUploadLyricsText(rawText, sourceLabel) }

  fun setLyricsOffsetMs(offsetMs: Long) =
    runEngineAction { SonoraNative.nativeSetLyricsOffsetMs(offsetMs) }

  fun fetchOnlineLyrics() {
    val info = _state.value.trackInfo ?: return
    val title = info.title.trim()
    val artist = info.artist.trim()
    if (title.isEmpty() && artist.isEmpty()) return

    _fetchingLyrics.value = true
    viewModelScope.launch(Dispatchers.IO) {
      try {
        val fetched = queryLrclib(artist, title)
        if (fetched != null) {
          SonoraNative.nativeUploadLyricsText(fetched.first, fetched.second)
          refreshSnapshotInternal()
        } else {
          showToast("NO ONLINE LYRICS FOUND ON LRCLIB")
        }
      } catch (e: Exception) {
        showToast("ONLINE LYRICS LOOKUP FAILED")
      } finally {
        _fetchingLyrics.value = false
      }
    }
  }

  private fun queryLrclib(artist: String, title: String): Pair<String, String>? {
    val encA = URLEncoder.encode(artist, "UTF-8")
    val encT = URLEncoder.encode(title, "UTF-8")
    // 1. Exact lookup
    httpGetString("https://lrclib.net/api/get?artist_name=$encA&track_name=$encT")?.let { body ->
      parseLrclibObject(JSONObject(body))?.let { return it }
    }
    // 2. Fallback search
    val q = URLEncoder.encode("$artist $title".trim(), "UTF-8")
    httpGetString("https://lrclib.net/api/search?q=$q")?.let { body ->
      val arr = JSONArray(body)
      for (i in 0 until arr.length()) {
        val obj = arr.optJSONObject(i) ?: continue
        parseLrclibObject(obj)?.let { return it }
      }
    }
    return null
  }

  private fun parseLrclibObject(obj: JSONObject): Pair<String, String>? {
    val synced = obj.optString("syncedLyrics", "").trim()
    if (synced.isNotEmpty() && synced != "null") return synced to "LRCLIB (Synced)"
    val plain = obj.optString("plainLyrics", "").trim()
    if (plain.isNotEmpty() && plain != "null") return plain to "LRCLIB (Plain)"
    return null
  }

  private fun httpGetString(urlStr: String): String? {
    val conn = (URL(urlStr).openConnection() as HttpURLConnection).apply {
      connectTimeout = 6000
      readTimeout = 6000
      setRequestProperty("User-Agent", "SonoraHiResPlayer/1.0")
    }
    return try {
      if (conn.responseCode in 200..299) {
        conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
      } else null
    } finally {
      conn.disconnect()
    }
  }

  private fun resolveDisplayName(uri: Uri): String? {
    appContext.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
      if (c.moveToFirst()) {
        val idx = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
        if (idx >= 0) return c.getString(idx)
      }
    }
    return uri.lastPathSegment
  }

  private fun shortHash(input: String): String {
    val md = MessageDigest.getInstance("SHA-256").digest(input.toByteArray())
    return md.take(5).joinToString("") { "%02x".format(it) }
  }

  private fun decodeTextAuto(bytes: ByteArray): String {
    if (bytes.size >= 2) {
      if (bytes[0] == 0xFF.toByte() && bytes[1] == 0xFE.toByte()) {
        return String(bytes, 2, bytes.size - 2, Charsets.UTF_16LE)
      }
      if (bytes[0] == 0xFE.toByte() && bytes[1] == 0xFF.toByte()) {
        return String(bytes, 2, bytes.size - 2, Charsets.UTF_16BE)
      }
    }
    return String(bytes, Charsets.UTF_8).removePrefix("\uFEFF")
  }
}
