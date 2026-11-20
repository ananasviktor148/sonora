package com.sonora.hiresplayer

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import android.os.Process
import java.util.concurrent.atomic.AtomicBoolean

class SonoraPlaybackService : Service() {

  companion object {
    const val CHANNEL_ID = "sonora_playback_channel"
    const val NOTIFICATION_ID = 1001

    const val ACTION_START = "com.sonora.hiresplayer.ACTION_START"
    const val ACTION_TOGGLE = "com.sonora.hiresplayer.ACTION_TOGGLE"
    const val ACTION_PREV = "com.sonora.hiresplayer.ACTION_PREV"
    const val ACTION_NEXT = "com.sonora.hiresplayer.ACTION_NEXT"
    const val ACTION_STOP = "com.sonora.hiresplayer.ACTION_STOP"
    const val ACTION_REFRESH = "com.sonora.hiresplayer.ACTION_REFRESH"

    fun sendAction(context: Context, action: String) {
      val intent = Intent(context, SonoraPlaybackService::class.java).apply { this.action = action }
      try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && action == ACTION_START) {
          context.startForegroundService(intent)
        } else {
          context.startService(intent)
        }
      } catch (_: Exception) {
        try {
          context.startService(intent)
        } catch (_: Exception) {}
      }
    }
  }

  private val running = AtomicBoolean(false)
  private var audioThread: Thread? = null
  private var mediaSession: MediaSession? = null
  private var audioManager: AudioManager? = null
  private var focusRequest: AudioFocusRequest? = null
  private var lastCoverRevision: Long = -1L
  private var cachedCover: Bitmap? = null
  private var noisyRegistered = false

  private val noisyReceiver = object : BroadcastReceiver() {
    override fun onReceive(context: Context?, intent: Intent?) {
      if (intent?.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
        SonoraNative.nativePause()
        updateNotificationAndSession()
      }
    }
  }

  private val audioFocusListener = AudioManager.OnAudioFocusChangeListener { change ->
    when (change) {
      AudioManager.AUDIOFOCUS_LOSS, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> {
        SonoraNative.nativePause()
        updateNotificationAndSession()
      }
    }
  }

  override fun onCreate() {
    super.onCreate()
    SonoraNative.nativeInit(filesDir.absolutePath)
    audioManager = getSystemService(Context.AUDIO_SERVICE) as AudioManager
    createNotificationChannel()
    setupMediaSession()

    val filter = IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      registerReceiver(noisyReceiver, filter, RECEIVER_NOT_EXPORTED)
    } else {
      registerReceiver(noisyReceiver, filter)
    }
    noisyRegistered = true

    startForegroundCompat(buildNotification(SonoraJsonParser.parseSnapshot(SonoraNative.nativeGetStateJson())))
    startAudioPumpThread()
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_TOGGLE -> {
        SonoraNative.nativeTogglePlayPause()
        requestAudioFocusIfPlaying()
      }
      ACTION_PREV -> {
        SonoraNative.nativePrev()
        requestAudioFocusIfPlaying()
      }
      ACTION_NEXT -> {
        SonoraNative.nativeNext(true)
        requestAudioFocusIfPlaying()
      }
      ACTION_STOP -> {
        SonoraNative.nativeStop()
        abandonAudioFocus()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
        return START_NOT_STICKY
      }
      ACTION_START, ACTION_REFRESH -> {
        requestAudioFocusIfPlaying()
      }
    }
    updateNotificationAndSession()
    return START_STICKY
  }

  override fun onDestroy() {
    running.set(false)
    audioThread?.interrupt()
    audioThread = null
    if (noisyRegistered) {
      try {
        unregisterReceiver(noisyReceiver)
      } catch (_: Exception) {}
      noisyRegistered = false
    }
    abandonAudioFocus()
    mediaSession?.isActive = false
    mediaSession?.release()
    mediaSession = null
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null

  private fun startForegroundCompat(notification: Notification) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
  }

  private fun createNotificationChannel() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val nm = getSystemService(NotificationManager::class.java)
      val channel = NotificationChannel(
        CHANNEL_ID,
        "Sonora Hi-Res Playback",
        NotificationManager.IMPORTANCE_LOW,
      ).apply {
        description = "High-resolution audio playback controls"
        setShowBadge(false)
        lockscreenVisibility = Notification.VISIBILITY_PUBLIC
      }
      nm.createNotificationChannel(channel)
    }
  }

  private fun setupMediaSession() {
    val session = MediaSession(this, "SonoraHiResSession")
    session.setCallback(object : MediaSession.Callback() {
      override fun onPlay() {
        SonoraNative.nativePlay()
        requestAudioFocusIfPlaying()
        updateNotificationAndSession()
      }

      override fun onPause() {
        SonoraNative.nativePause()
        updateNotificationAndSession()
      }

      override fun onSkipToNext() {
        SonoraNative.nativeNext(true)
        requestAudioFocusIfPlaying()
        updateNotificationAndSession()
      }

      override fun onSkipToPrevious() {
        SonoraNative.nativePrev()
        requestAudioFocusIfPlaying()
        updateNotificationAndSession()
      }

      override fun onStop() {
        SonoraNative.nativeStop()
        abandonAudioFocus()
        updateNotificationAndSession()
      }

      override fun onSeekTo(pos: Long) {
        SonoraNative.nativeSeek(pos / 1000.0)
        updateNotificationAndSession()
      }
    })
    session.isActive = true
    mediaSession = session
  }

  private fun requestAudioFocusIfPlaying() {
    val snap = SonoraJsonParser.parseSnapshot(SonoraNative.nativeGetStateJson())
    if (!snap.isPlaying) return
    val am = audioManager ?: return
    val attrs = AudioAttributes.Builder()
      .setUsage(AudioAttributes.USAGE_MEDIA)
      .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
      .build()
    val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
      .setAudioAttributes(attrs)
      .setOnAudioFocusChangeListener(audioFocusListener)
      .build()
    focusRequest = req
    am.requestAudioFocus(req)
  }

  private fun abandonAudioFocus() {
    val am = audioManager ?: return
    focusRequest?.let { am.abandonAudioFocusRequest(it) }
    focusRequest = null
  }

  private fun openAudioTrack(desiredRate: Int): Pair<AudioTrack, Int>? {
    val candidates = linkedSetOf(desiredRate.coerceIn(44_100, 192_000), 192_000, 96_000, 48_000, 44_100)
      .filter { it <= desiredRate.coerceAtLeast(48_000) || it == 48_000 }
    val attrs = AudioAttributes.Builder()
      .setUsage(AudioAttributes.USAGE_MEDIA)
      .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
      .build()

    for (rate in candidates) {
      try {
        val format = AudioFormat.Builder()
          .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
          .setSampleRate(rate)
          .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
          .build()
        val minBuf = AudioTrack.getMinBufferSize(
          rate,
          AudioFormat.CHANNEL_OUT_STEREO,
          AudioFormat.ENCODING_PCM_FLOAT,
        )
        if (minBuf <= 0) continue
        val bufBytes = (minBuf * 2).coerceAtLeast(16_384)
        val builder = AudioTrack.Builder()
          .setAudioAttributes(attrs)
          .setAudioFormat(format)
          .setTransferMode(AudioTrack.MODE_STREAM)
          .setBufferSizeInBytes(bufBytes)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
          builder.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
        }
        val track = builder.build()
        if (track.state == AudioTrack.STATE_INITIALIZED) {
          return track to rate
        }
        track.release()
      } catch (_: Exception) {
        // Try next supported rate
      }
    }
    return null
  }

  private fun startAudioPumpThread() {
    if (!running.compareAndSet(false, true)) return
    audioThread = Thread({
      Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
      var track: AudioTrack? = null
      var openedRate = 0
      val pcmBuf = FloatArray(4096) // 2048 stereo frames per render slice
      var lastNotifAt = 0L

      try {
        while (running.get()) {
          val desired = SonoraNative.nativeGetDesiredOutputRate().coerceIn(44_100, 192_000)
          if (track == null || openedRate != desired) {
            track?.let {
              try {
                it.pause()
                it.flush()
                it.release()
              } catch (_: Exception) {}
            }
            val opened = openAudioTrack(desired)
            if (opened != null) {
              track = opened.first
              openedRate = opened.second
              SonoraNative.nativeSetActualOutputFormat(openedRate, 2)
              track.play()
            } else {
              Thread.sleep(40)
              continue
            }
          }

          val written = SonoraNative.nativeRenderAudio(pcmBuf)
          val activeTrack = track
          when {
            written > 0 -> {
              if (activeTrack.playState != AudioTrack.PLAYSTATE_PLAYING) {
                activeTrack.play()
              }
              activeTrack.write(pcmBuf, 0, written, AudioTrack.WRITE_BLOCKING)
              val now = System.currentTimeMillis()
              if (now - lastNotifAt > 1500L) {
                lastNotifAt = now
                updateNotificationAndSession()
              }
            }
            written == -1 -> {
              // Track finished cleanly — advance playlist automatically even with screen off
              SonoraNative.nativeNext(false)
              updateNotificationAndSession()
            }
            else -> {
              if (activeTrack.playState == AudioTrack.PLAYSTATE_PLAYING) {
                activeTrack.pause()
                activeTrack.flush()
              }
              Thread.sleep(20)
            }
          }
        }
      } catch (_: InterruptedException) {
        // Clean shutdown
      } finally {
        track?.let {
          try {
            it.pause()
            it.flush()
            it.release()
          } catch (_: Exception) {}
        }
      }
    }, "sonora-audiotrack-pump").apply { start() }
  }

  private fun updateNotificationAndSession() {
    val snap = SonoraJsonParser.parseSnapshot(SonoraNative.nativeGetStateJson())
    if (snap.coverRevision != lastCoverRevision) {
      lastCoverRevision = snap.coverRevision
      val raw = SonoraNative.nativeGetCoverArtBytes()
      cachedCover = if (raw.isNotEmpty()) BitmapFactory.decodeByteArray(raw, 0, raw.size) else null
    }

    mediaSession?.let { session ->
      val info = snap.trackInfo
      val metaBuilder = MediaMetadata.Builder()
        .putString(MediaMetadata.METADATA_KEY_TITLE, info?.title ?: "Sonora Hi-Res")
        .putString(MediaMetadata.METADATA_KEY_ARTIST, info?.artist?.ifBlank { "Hi-Res Audio Studio" } ?: "Hi-Res Audio Studio")
        .putString(MediaMetadata.METADATA_KEY_ALBUM, info?.album ?: "")
        .putLong(MediaMetadata.METADATA_KEY_DURATION, (snap.duration * 1000.0).toLong())
      cachedCover?.let { metaBuilder.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) }
      session.setMetadata(metaBuilder.build())

      val pbState = when (snap.state) {
        "Playing" -> PlaybackState.STATE_PLAYING
        "Paused" -> PlaybackState.STATE_PAUSED
        else -> PlaybackState.STATE_STOPPED
      }
      val actions = PlaybackState.ACTION_PLAY or
        PlaybackState.ACTION_PAUSE or
        PlaybackState.ACTION_PLAY_PAUSE or
        PlaybackState.ACTION_SKIP_TO_NEXT or
        PlaybackState.ACTION_SKIP_TO_PREVIOUS or
        PlaybackState.ACTION_SEEK_TO or
        PlaybackState.ACTION_STOP
      val stateObj = PlaybackState.Builder()
        .setActions(actions)
        .setState(pbState, (snap.position * 1000.0).toLong(), if (snap.isPlaying) 1.0f else 0.0f)
        .build()
      session.setPlaybackState(stateObj)
    }

    val nm = getSystemService(NotificationManager::class.java)
    nm.notify(NOTIFICATION_ID, buildNotification(snap))
  }

  private fun buildNotification(snap: SonoraState): Notification {
    val openIntent = Intent(this, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    val contentPi = PendingIntent.getActivity(
      this, 0, openIntent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    fun servicePi(action: String, code: Int): PendingIntent {
      val i = Intent(this, SonoraPlaybackService::class.java).apply { this.action = action }
      return PendingIntent.getService(this, code, i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }

    val info = snap.trackInfo
    val title = info?.title ?: "Sonora Hi-Res Audio"
    val subtitle = buildString {
      if (info != null) {
        if (info.artist.isNotBlank()) {
          append(info.artist)
          append(" · ")
        }
        if (info.isDsd) append(info.codec) else append("${info.format} ${formatRate(snap.outputSampleRate)}")
      } else {
        append("Ready")
      }
    }

    val playPauseIcon = if (snap.isPlaying) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play
    val playPauseLabel = if (snap.isPlaying) "Pause" else "Play"

    val builder = Notification.Builder(this, CHANNEL_ID)
      .setContentTitle(title)
      .setContentText(subtitle)
      .setSmallIcon(android.R.drawable.ic_media_play)
      .setContentIntent(contentPi)
      .setOngoing(snap.isPlaying)
      .setVisibility(Notification.VISIBILITY_PUBLIC)
      .addAction(
        Notification.Action.Builder(
          Icon.createWithResource(this, android.R.drawable.ic_media_previous),
          "Previous",
          servicePi(ACTION_PREV, 1),
        ).build()
      )
      .addAction(
        Notification.Action.Builder(
          Icon.createWithResource(this, playPauseIcon),
          playPauseLabel,
          servicePi(ACTION_TOGGLE, 2),
        ).build()
      )
      .addAction(
        Notification.Action.Builder(
          Icon.createWithResource(this, android.R.drawable.ic_media_next),
          "Next",
          servicePi(ACTION_NEXT, 3),
        ).build()
      )

    cachedCover?.let { builder.setLargeIcon(it) }
    mediaSession?.sessionToken?.let { token ->
      builder.setStyle(
        Notification.MediaStyle()
          .setMediaSession(token)
          .setShowActionsInCompactView(0, 1, 2)
      )
    }
    return builder.build()
  }
}
