package com.sonora.hiresplayer

object SonoraNative {
  init {
    System.loadLibrary("sonora_core")
  }

  external fun nativeInit(filesDir: String)
  external fun nativeGetStateJson(): String
  external fun nativeRenderAudio(outBuf: FloatArray): Int
  external fun nativeGetDesiredOutputRate(): Int
  external fun nativeSetActualOutputFormat(sampleRate: Int, channels: Int)
  external fun nativeGetVisualizerFloats(): FloatArray
  external fun nativeGetCoverArtBytes(): ByteArray

  external fun nativePlay()
  external fun nativePause()
  external fun nativeTogglePlayPause()
  external fun nativeStop()
  external fun nativeSeek(seconds: Double)
  external fun nativeNext(userTriggered: Boolean)
  external fun nativePrev()
  external fun nativeLoadTrack(playlistId: String, index: Int, autoplay: Boolean)

  external fun nativeSetVolume(vol: Float)
  external fun nativeSetMuted(muted: Boolean)
  external fun nativeToggleShuffle()
  external fun nativeCycleRepeat()
  external fun nativeUpdateConfigJson(json: String)

  external fun nativeCycleAutoVolume()
  external fun nativeReanalyzeAutoVolume()
  external fun nativeSetChannelMuteSolo(channel: Int, mute: Boolean, solo: Boolean)
  external fun nativeResetMuteSolo()

  external fun nativeImportPathsJson(pathsJson: String): Int
  external fun nativeCreatePlaylist(name: String, copyCurrent: Boolean, seedIndex: Int)
  external fun nativeSwitchPlaylist(id: String)
  external fun nativeRenamePlaylist(id: String, newName: String)
  external fun nativeDuplicatePlaylist(id: String)
  external fun nativeDeletePlaylist(id: String)
  external fun nativeAddTrackToPlaylist(targetId: String, sourceIndex: Int)
  external fun nativeRemoveTrack(index: Int)
  external fun nativeMoveTrack(from: Int, to: Int)
  external fun nativeClearActivePlaylist()
  external fun nativeRestoreStudioReferences()

  external fun nativeUploadLyricsText(rawText: String, sourceLabel: String)
  external fun nativeSetLyricsOffsetMs(offsetMs: Long)
  external fun nativeGetLyricsRaw(): String
}
