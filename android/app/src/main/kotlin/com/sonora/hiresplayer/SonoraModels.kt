package com.sonora.hiresplayer

import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.roundToInt

data class TrackInfoUi(
  val title: String = "",
  val artist: String = "",
  val album: String = "",
  val duration: Double = 0.0,
  val sampleRate: Int = 0,
  val bitDepth: Int = 16,
  val channels: Int = 2,
  val channelLabels: List<String> = listOf("FL", "FR"),
  val channelLayout: String = "Stereo (2.0)",
  val format: String = "",
  val codec: String = "",
  val isDsd: Boolean = false,
  val isHires: Boolean = false,
  val isDemo: Boolean = false,
  val decoderBackend: String = "",
  val filePath: String = "",
)

data class SignalPathStageUi(
  val title: String,
  val detail: String,
  val lossless: Boolean,
)

data class SignalPathUi(
  val stages: List<SignalPathStageUi> = emptyList(),
  val bitPerfect: Boolean = false,
)

data class AutoVolumeUi(
  val active: Boolean = false,
  val analyzing: Boolean = false,
  val mode: String = "Off",
  val appliedGainDb: Float = 0f,
  val measuredLufs: Float? = null,
  val peakDbfs: Float? = null,
  val peakLimited: Boolean = false,
  val bypassedForDop: Boolean = false,
  val sourceLabel: String = "Off",
)

data class PlaylistEntryUi(
  val path: String,
  val title: String,
  val artist: String,
  val album: String,
  val duration: Double,
  val sampleRate: Int,
  val bitDepth: Int,
  val channels: Int,
  val format: String,
  val codec: String,
  val isDsd: Boolean,
  val isHires: Boolean,
  val isDemo: Boolean,
) {
  fun specCode(): String {
    if (isDsd) {
      return if (channels > 2) "$codec · ${channels}CH" else codec
    }
    if (sampleRate > 0) {
      val sr = if (sampleRate >= 1000) {
        val k = sampleRate / 1000.0
        if (kotlin.math.abs(k - k.roundToInt()) < 0.05) "${k.roundToInt()}K" else String.format("%.1fK", k)
      } else {
        "${sampleRate}HZ"
      }
      return if (channels > 2) "$format $sr/$bitDepth · ${channels}CH" else "$format $sr/$bitDepth"
    }
    return format
  }
}

data class PlaylistUi(
  val id: String,
  val name: String,
  val tracks: List<PlaylistEntryUi>,
) {
  val totalDuration: Double get() = tracks.sumOf { it.duration }
}

data class SyncedLineUi(
  val timeSecs: Double,
  val text: String,
)

data class LyricsUi(
  val rawText: String,
  val syncedLines: List<SyncedLineUi>,
  val plainLines: List<String>,
  val source: String,
  val tagOffsetMs: Long,
) {
  val isSynced: Boolean get() = syncedLines.isNotEmpty()

  fun activeLineIndex(positionSecs: Double, userOffsetMs: Long): Int {
    if (syncedLines.isEmpty()) return -1
    val adjusted = positionSecs + (tagOffsetMs + userOffsetMs) / 1000.0
    if (adjusted < syncedLines[0].timeSecs) return -1
    var lo = 0
    var hi = syncedLines.lastIndex
    var ans = 0
    while (lo <= hi) {
      val mid = (lo + hi) ushr 1
      if (syncedLines[mid].timeSecs <= adjusted) {
        ans = mid
        lo = mid + 1
      } else {
        hi = mid - 1
      }
    }
    return ans
  }
}

data class ConfigUi(
  val volume: Float = 0.8f,
  val headroomDb: Float = -1.0f,
  val softClip: Boolean = true,
  val upsampleMode: String = "Bypass", // "Bypass", "Factor2x", "Factor4x", "Factor8x", "Fixed"
  val upsampleFixedRate: Int = 192_000,
  val resamplerQuality: String = "Audiophile",
  val filterRolloff: String = "Sharp",
  val dsdOutputMode: String = "Pcm176k",
  val dsdCutoffHz: Int = 30_000,
  val dsdGainBoost6db: Boolean = true,
  val downmixMode: String = "Auto",
  val includeLfe: Boolean = true,
  val lfeGainDb: Float = -6.0f,
  val centerGainDb: Float = -3.0f,
  val surroundGainDb: Float = -3.0f,
  val crossfeedAmount: Float = 0.25f,
  val eqEnabled: Boolean = false,
  val eqPreampDb: Float = 0.0f,
  val eqBandsDb: List<Float> = List(10) { 0f },
  val eqPresetName: String = "Flat",
  val autoVolMode: String = "Off",
  val autoVolTargetLufs: Float = -16.0f,
  val autoVolPreventClipping: Boolean = true,
  val autoVolPreferRgTags: Boolean = true,
  val lyricsFontSize: Float = 16.0f,
  val lyricsAutoScroll: Boolean = true,
  val shuffle: Boolean = false,
  val repeatMode: String = "Off",
) {
  fun toRustJson(): String {
    val root = JSONObject()
    root.put("volume", volume.toDouble())
    root.put("headroom_db", headroomDb.toDouble())
    root.put("soft_clip", softClip)
    root.put("selected_device", JSONObject.NULL)
    if (upsampleMode == "Fixed") {
      root.put("upsample_mode", JSONObject().put("Fixed", upsampleFixedRate))
    } else {
      root.put("upsample_mode", upsampleMode)
    }
    root.put("resampler_quality", resamplerQuality)
    root.put("filter_rolloff", filterRolloff)

    val dsd = JSONObject()
    dsd.put("output_mode", dsdOutputMode)
    dsd.put("cutoff_hz", dsdCutoffHz)
    dsd.put("gain_boost_6db", dsdGainBoost6db)
    root.put("dsd", dsd)

    val mc = JSONObject()
    mc.put("downmix_mode", downmixMode)
    mc.put("include_lfe_in_downmix", includeLfe)
    mc.put("lfe_gain_db", lfeGainDb.toDouble())
    mc.put("center_gain_db", centerGainDb.toDouble())
    mc.put("surround_gain_db", surroundGainDb.toDouble())
    mc.put("crossfeed_amount", crossfeedAmount.toDouble())
    root.put("multichannel", mc)

    val eq = JSONObject()
    eq.put("enabled", eqEnabled)
    eq.put("preamp_db", eqPreampDb.toDouble())
    val bands = JSONArray()
    for (i in 0 until 10) bands.put(eqBandsDb.getOrElse(i) { 0f }.toDouble())
    eq.put("bands_db", bands)
    eq.put("preset_name", eqPresetName)
    root.put("eq", eq)

    val av = JSONObject()
    av.put("mode", autoVolMode)
    av.put("target_lufs", autoVolTargetLufs.toDouble())
    av.put("prevent_clipping", autoVolPreventClipping)
    av.put("prefer_replaygain_tags", autoVolPreferRgTags)
    root.put("auto_volume", av)

    root.put("lyrics_font_size", lyricsFontSize.toDouble())
    root.put("lyrics_auto_scroll", lyricsAutoScroll)
    root.put("shuffle", shuffle)
    root.put("repeat_mode", repeatMode)
    return root.toString()
  }
}

data class SonoraState(
  val state: String = "Stopped",
  val position: Double = 0.0,
  val duration: Double = 0.0,
  val sourceSampleRate: Int = 0,
  val outputSampleRate: Int = 48_000,
  val desiredOutputRate: Int = 48_000,
  val sourceChannels: Int = 2,
  val outputChannels: Int = 2,
  val upsampled: Boolean = false,
  val downmixed: Boolean = false,
  val muted: Boolean = false,
  val trackInfo: TrackInfoUi? = null,
  val autoVolume: AutoVolumeUi = AutoVolumeUi(),
  val signalPath: SignalPathUi = SignalPathUi(),
  val config: ConfigUi = ConfigUi(),
  val channelMuted: List<Boolean> = List(8) { false },
  val channelSolo: List<Boolean> = List(8) { false },
  val activePlaylistId: String = "",
  val playingPlaylistId: String? = null,
  val currentIndex: Int? = null,
  val playlists: List<PlaylistUi> = emptyList(),
  val lyrics: LyricsUi? = null,
  val lyricsOffsetMs: Long = 0L,
  val coverRevision: Long = 0L,
  val errorMessage: String? = null,
  val toast: String? = null,
) {
  val isPlaying: Boolean get() = state == "Playing"
  val activePlaylist: PlaylistUi? get() = playlists.firstOrNull { it.id == activePlaylistId } ?: playlists.firstOrNull()
}

object SonoraJsonParser {
  fun parseSnapshot(raw: String): SonoraState {
    if (raw.isBlank() || raw == "{}") return SonoraState()
    return try {
      val root = JSONObject(raw)
      val trackObj = root.optJSONObject("track_info")
      val trackInfo = trackObj?.let { t ->
        val labelsArr = t.optJSONArray("channel_labels")
        val labels = buildList {
          if (labelsArr != null) {
            for (i in 0 until labelsArr.length()) add(labelsArr.optString(i))
          }
        }.ifEmpty { listOf("FL", "FR") }
        TrackInfoUi(
          title = t.optString("title"),
          artist = t.optString("artist"),
          album = t.optString("album"),
          duration = t.optDouble("duration", 0.0),
          sampleRate = t.optInt("sample_rate", 0),
          bitDepth = t.optInt("bit_depth", 16),
          channels = t.optInt("channels", 2),
          channelLabels = labels,
          channelLayout = t.optString("channel_layout", "Stereo (2.0)"),
          format = t.optString("format"),
          codec = t.optString("codec"),
          isDsd = t.optBoolean("is_dsd", false),
          isHires = t.optBoolean("is_hires", false),
          isDemo = t.optBoolean("is_demo", false),
          decoderBackend = t.optString("decoder_backend"),
          filePath = t.optString("file_path"),
        )
      }

      val avObj = root.optJSONObject("auto_volume")
      val autoVol = if (avObj != null) {
        AutoVolumeUi(
          active = avObj.optBoolean("active", false),
          analyzing = avObj.optBoolean("analyzing", false),
          mode = avObj.optString("mode", "Off"),
          appliedGainDb = avObj.optDouble("applied_gain_db", 0.0).toFloat(),
          measuredLufs = if (avObj.isNull("measured_lufs")) null else avObj.optDouble("measured_lufs").toFloat(),
          peakDbfs = if (avObj.isNull("peak_dbfs")) null else avObj.optDouble("peak_dbfs").toFloat(),
          peakLimited = avObj.optBoolean("peak_limited", false),
          bypassedForDop = avObj.optBoolean("bypassed_for_dop", false),
          sourceLabel = avObj.optString("source_label", "Off"),
        )
      } else AutoVolumeUi()

      val spObj = root.optJSONObject("signal_path")
      val stages = buildList {
        val arr = spObj?.optJSONArray("stages")
        if (arr != null) {
          for (i in 0 until arr.length()) {
            val s = arr.optJSONObject(i) ?: continue
            add(
              SignalPathStageUi(
                title = s.optString("title"),
                detail = s.optString("detail"),
                lossless = s.optBoolean("lossless", true),
              )
            )
          }
        }
      }
      val signalPath = SignalPathUi(
        stages = stages,
        bitPerfect = spObj?.optBoolean("bit_perfect", false) ?: false,
      )

      val cfgObj = root.optJSONObject("config")
      val config = if (cfgObj != null) parseConfig(cfgObj) else ConfigUi()

      val chMuted = parseBoolList(root.optJSONArray("channel_muted"), 8)
      val chSolo = parseBoolList(root.optJSONArray("channel_solo"), 8)

      val playlists = buildList {
        val arr = root.optJSONArray("playlists")
        if (arr != null) {
          for (i in 0 until arr.length()) {
            val p = arr.optJSONObject(i) ?: continue
            val tArr = p.optJSONArray("tracks")
            val tracks = buildList {
              if (tArr != null) {
                for (j in 0 until tArr.length()) {
                  val e = tArr.optJSONObject(j) ?: continue
                  add(
                    PlaylistEntryUi(
                      path = e.optString("path"),
                      title = e.optString("title"),
                      artist = e.optString("artist"),
                      album = e.optString("album"),
                      duration = e.optDouble("duration", 0.0),
                      sampleRate = e.optInt("sample_rate", 0),
                      bitDepth = e.optInt("bit_depth", 16),
                      channels = e.optInt("channels", 2),
                      format = e.optString("format"),
                      codec = e.optString("codec"),
                      isDsd = e.optBoolean("is_dsd", false),
                      isHires = e.optBoolean("is_hires", false),
                      isDemo = e.optBoolean("is_demo", false),
                    )
                  )
                }
              }
            }
            add(PlaylistUi(id = p.optString("id"), name = p.optString("name"), tracks = tracks))
          }
        }
      }

      val lyrObj = root.optJSONObject("lyrics")
      val lyrics = lyrObj?.let { l ->
        val synced = buildList {
          val arr = l.optJSONArray("synced_lines")
          if (arr != null) {
            for (i in 0 until arr.length()) {
              val item = arr.optJSONObject(i) ?: continue
              add(SyncedLineUi(item.optDouble("time_secs", 0.0), item.optString("text")))
            }
          }
        }
        val plain = buildList {
          val arr = l.optJSONArray("plain_lines")
          if (arr != null) {
            for (i in 0 until arr.length()) add(arr.optString(i))
          }
        }
        LyricsUi(
          rawText = l.optString("raw_text"),
          syncedLines = synced,
          plainLines = plain,
          source = l.optString("source"),
          tagOffsetMs = l.optLong("tag_offset_ms", 0L),
        )
      }

      SonoraState(
        state = root.optString("state", "Stopped"),
        position = root.optDouble("position", 0.0),
        duration = root.optDouble("duration", 0.0),
        sourceSampleRate = root.optInt("source_sample_rate", 0),
        outputSampleRate = root.optInt("output_sample_rate", 48_000),
        desiredOutputRate = root.optInt("desired_output_rate", 48_000),
        sourceChannels = root.optInt("source_channels", 2),
        outputChannels = root.optInt("output_channels", 2),
        upsampled = root.optBoolean("upsampled", false),
        downmixed = root.optBoolean("downmixed", false),
        muted = root.optBoolean("muted", false),
        trackInfo = trackInfo,
        autoVolume = autoVol,
        signalPath = signalPath,
        config = config,
        channelMuted = chMuted,
        channelSolo = chSolo,
        activePlaylistId = root.optString("active_playlist_id"),
        playingPlaylistId = if (root.isNull("playing_playlist_id")) null else root.optString("playing_playlist_id"),
        currentIndex = if (root.isNull("current_index")) null else root.optInt("current_index"),
        playlists = playlists,
        lyrics = lyrics,
        lyricsOffsetMs = root.optLong("lyrics_offset_ms", 0L),
        coverRevision = root.optLong("cover_revision", 0L),
        errorMessage = if (root.isNull("error_message")) null else root.optString("error_message"),
        toast = if (root.isNull("toast")) null else root.optString("toast"),
      )
    } catch (_: Exception) {
      SonoraState()
    }
  }

  private fun parseBoolList(arr: JSONArray?, len: Int): List<Boolean> =
    List(len) { idx -> arr?.optBoolean(idx, false) ?: false }

  private fun parseConfig(c: JSONObject): ConfigUi {
    val upRaw = c.opt("upsample_mode")
    var upMode = "Bypass"
    var upFixed = 192_000
    when (upRaw) {
      is String -> upMode = upRaw
      is JSONObject -> {
        if (upRaw.has("Fixed")) {
          upMode = "Fixed"
          upFixed = upRaw.optInt("Fixed", 192_000)
        }
      }
    }

    val dsd = c.optJSONObject("dsd")
    val mc = c.optJSONObject("multichannel")
    val eq = c.optJSONObject("eq")
    val av = c.optJSONObject("auto_volume")
    val bandsArr = eq?.optJSONArray("bands_db")
    val bands = List(10) { i -> bandsArr?.optDouble(i, 0.0)?.toFloat() ?: 0f }

    return ConfigUi(
      volume = c.optDouble("volume", 0.8).toFloat(),
      headroomDb = c.optDouble("headroom_db", -1.0).toFloat(),
      softClip = c.optBoolean("soft_clip", true),
      upsampleMode = upMode,
      upsampleFixedRate = upFixed,
      resamplerQuality = c.optString("resampler_quality", "Audiophile"),
      filterRolloff = c.optString("filter_rolloff", "Sharp"),
      dsdOutputMode = dsd?.optString("output_mode", "Pcm176k") ?: "Pcm176k",
      dsdCutoffHz = dsd?.optInt("cutoff_hz", 30_000) ?: 30_000,
      dsdGainBoost6db = dsd?.optBoolean("gain_boost_6db", true) ?: true,
      downmixMode = mc?.optString("downmix_mode", "Auto") ?: "Auto",
      includeLfe = mc?.optBoolean("include_lfe_in_downmix", true) ?: true,
      lfeGainDb = mc?.optDouble("lfe_gain_db", -6.0)?.toFloat() ?: -6.0f,
      centerGainDb = mc?.optDouble("center_gain_db", -3.0)?.toFloat() ?: -3.0f,
      surroundGainDb = mc?.optDouble("surround_gain_db", -3.0)?.toFloat() ?: -3.0f,
      crossfeedAmount = mc?.optDouble("crossfeed_amount", 0.25)?.toFloat() ?: 0.25f,
      eqEnabled = eq?.optBoolean("enabled", false) ?: false,
      eqPreampDb = eq?.optDouble("preamp_db", 0.0)?.toFloat() ?: 0.0f,
      eqBandsDb = bands,
      eqPresetName = eq?.optString("preset_name", "Flat") ?: "Flat",
      autoVolMode = av?.optString("mode", "Off") ?: "Off",
      autoVolTargetLufs = av?.optDouble("target_lufs", -16.0)?.toFloat() ?: -16.0f,
      autoVolPreventClipping = av?.optBoolean("prevent_clipping", true) ?: true,
      autoVolPreferRgTags = av?.optBoolean("prefer_replaygain_tags", true) ?: true,
      lyricsFontSize = c.optDouble("lyrics_font_size", 16.0).toFloat(),
      lyricsAutoScroll = c.optBoolean("lyrics_auto_scroll", true),
      shuffle = c.optBoolean("shuffle", false),
      repeatMode = c.optString("repeat_mode", "Off"),
    )
  }
}

fun formatTime(seconds: Double): String {
  val total = seconds.coerceAtLeast(0.0).toLong()
  val h = total / 3600
  val m = (total % 3600) / 60
  val s = total % 60
  return if (h > 0) String.format("%d:%02d:%02d", h, m, s) else String.format("%02d:%02d", m, s)
}

fun formatRate(rate: Int): String = when {
  rate >= 1_000_000 -> String.format("%.2f MHz", rate / 1_000_000.0)
  rate >= 1_000 -> {
    val khz = rate / 1_000.0
    if (kotlin.math.abs(khz - khz.roundToInt()) < 0.02) "${khz.roundToInt()} kHz" else String.format("%.1f kHz", khz)
  }
  rate > 0 -> "$rate Hz"
  else -> "—"
}
