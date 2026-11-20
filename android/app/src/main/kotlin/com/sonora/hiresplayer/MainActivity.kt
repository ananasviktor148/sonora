package com.sonora.hiresplayer

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.sonora.hiresplayer.ui.*
import com.sonora.hiresplayer.ui.theme.*

private enum class MobileTab(val code: String, val label: String) {
  Playing("01", "PLAYING"),
  Playlists("02", "LISTS"),
  Lyrics("03", "LYRICS"),
  Dsp("04", "DSP/DSD"),
  MixEq("05", "MIX/EQ"),
}

class MainActivity : ComponentActivity() {

  private val notifPermLauncher =
    registerForActivityResult(ActivityResultContracts.RequestPermission()) { /* optional */ }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)

    val darkArgb = Ground.toArgb()
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(darkArgb),
      navigationBarStyle = SystemBarStyle.dark(darkArgb),
    )

    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
      ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
      PackageManager.PERMISSION_GRANTED
    ) {
      notifPermLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    setContent {
      SonoraTheme {
        SonoraAppRoot()
      }
    }
  }
}

@Composable
private fun SonoraAppRoot(vm: SonoraViewModel = viewModel()) {
  val state by vm.state.collectAsStateWithLifecycle()
  val visualizer by vm.visualizer.collectAsStateWithLifecycle()
  val coverBitmap by vm.coverBitmap.collectAsStateWithLifecycle()
  val toast by vm.toastMessage.collectAsStateWithLifecycle()
  val fetchingLyrics by vm.fetchingLyrics.collectAsStateWithLifecycle()

  var activeTab by remember { mutableStateOf(MobileTab.Playing) }

  val audioPicker = rememberLauncherForActivityResult(
    contract = ActivityResultContracts.OpenMultipleDocuments(),
  ) { uris ->
    if (uris.isNotEmpty()) {
      vm.importAudioUris(uris)
    }
  }

  val lyricsPicker = rememberLauncherForActivityResult(
    contract = ActivityResultContracts.OpenDocument(),
  ) { uri ->
    if (uri != null) {
      vm.importLyricsUri(uri)
    }
  }

  val openAudioFiles = { audioPicker.launch(arrayOf("*/*")) }
  val openLyricsFile = { lyricsPicker.launch(arrayOf("*/*")) }

  Scaffold(
    containerColor = Ground,
    topBar = {
      Column(
        modifier = Modifier
          .fillMaxWidth()
          .background(Surface)
          .statusBarsPadding(),
      ) {
        Row(
          modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 10.dp),
          verticalAlignment = Alignment.CenterVertically,
        ) {
          Box(
            modifier = Modifier
              .size(8.dp)
              .clip(RoundedCornerShape(2.dp))
              .background(Signal),
          )
          Spacer(Modifier.width(8.dp))
          Text(
            text = "SONORA",
            fontFamily = CleanSans,
            fontSize = 14.sp,
            fontWeight = FontWeight.Bold,
            color = Ink,
          )
          Spacer(Modifier.width(6.dp))
          Text(
            text = "№ 01",
            fontFamily = TechnicalMono,
            fontSize = 10.sp,
            color = InkMute,
          )
          Spacer(Modifier.weight(1f))

          state.trackInfo?.let { info ->
            val pill = if (info.isDsd) info.codec else "${info.format} ${formatRate(state.outputSampleRate)}"
            Box(
              modifier = Modifier
                .clip(RoundedCornerShape(4.dp))
                .background(Ground)
                .border(1.dp, Rule, RoundedCornerShape(4.dp))
                .padding(horizontal = 8.dp, vertical = 4.dp),
            ) {
              Text(
                text = pill,
                fontFamily = TechnicalMono,
                fontSize = 9.5.sp,
                color = if (info.isDsd) Brass else if (state.upsampled) Signal else Mint,
              )
            }
            Spacer(Modifier.width(8.dp))
          }

          SwissPrimaryButton("+ FILES", onClick = openAudioFiles)
        }
        SwissHairline()
      }
    },
    bottomBar = {
      Column(
        modifier = Modifier
          .fillMaxWidth()
          .background(Surface)
          .navigationBarsPadding(),
      ) {
        // Persistent Mini-Transport Bar when browsing non-Playing tabs
        if (activeTab != MobileTab.Playing && state.trackInfo != null) {
          val info = state.trackInfo!!
          SwissHairline()
          Row(
            modifier = Modifier
              .fillMaxWidth()
              .background(SurfaceAlt)
              .clickable { activeTab = MobileTab.Playing }
              .padding(horizontal = 14.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
          ) {
            Column(modifier = Modifier.weight(1f)) {
              Text(
                text = info.title,
                fontSize = 12.5.sp,
                fontWeight = FontWeight.SemiBold,
                color = Ink,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
              )
              Text(
                text = "${info.artist.ifBlank { info.format }} · ${formatTime(state.position)} / ${formatTime(state.duration)}",
                fontFamily = TechnicalMono,
                fontSize = 9.5.sp,
                color = InkMute,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
              )
            }
            Spacer(Modifier.width(8.dp))
            SwissGhostButton("⏮", onClick = { vm.prevTrack() })
            Spacer(Modifier.width(6.dp))
            SwissPrimaryButton(
              label = if (state.isPlaying) "⏸" else "▶",
              onClick = { vm.togglePlayPause() },
              fillColor = if (state.isPlaying) Ink else Signal,
            )
            Spacer(Modifier.width(6.dp))
            SwissGhostButton("⏭", onClick = { vm.nextTrack() })
          }
        }

        SwissHairline()
        Row(
          modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 6.dp, vertical = 6.dp),
          horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
          MobileTab.entries.forEach { tab ->
            val selected = activeTab == tab
            Column(
              horizontalAlignment = Alignment.CenterHorizontally,
              modifier = Modifier
                .weight(1f)
                .clip(RoundedCornerShape(4.dp))
                .background(if (selected) SurfaceAlt else androidx.compose.ui.graphics.Color.Transparent)
                .border(
                  1.dp,
                  if (selected) Signal else androidx.compose.ui.graphics.Color.Transparent,
                  RoundedCornerShape(4.dp),
                )
                .clickable { activeTab = tab }
                .padding(vertical = 7.dp),
            ) {
              Text(
                text = tab.code,
                fontFamily = TechnicalMono,
                fontSize = 9.sp,
                fontWeight = FontWeight.Bold,
                color = if (selected) Signal else InkMute,
              )
              Text(
                text = tab.label,
                fontFamily = TechnicalMono,
                fontSize = 9.5.sp,
                fontWeight = if (selected) FontWeight.Bold else FontWeight.Medium,
                color = if (selected) Ink else InkDim,
                maxLines = 1,
              )
            }
          }
        }
      }
    },
  ) { innerPadding ->
    Box(
      modifier = Modifier
        .fillMaxSize()
        .padding(innerPadding),
    ) {
      when (activeTab) {
        MobileTab.Playing -> NowPlayingScreen(
          state = state,
          visualizer = visualizer,
          coverBitmap = coverBitmap,
          onTogglePlayPause = { vm.togglePlayPause() },
          onStop = { vm.stop() },
          onPrev = { vm.prevTrack() },
          onNext = { vm.nextTrack() },
          onSeek = { vm.seekTo(it) },
          onToggleShuffle = { vm.toggleShuffle() },
          onCycleRepeat = { vm.cycleRepeat() },
          onSetVolume = { vm.setVolume(it) },
          onToggleMute = { vm.toggleMute() },
          onCycleAutoVolume = { vm.cycleAutoVolume() },
          onOpenFiles = openAudioFiles,
          onRestoreReferences = { vm.restoreStudioReferences() },
          onJumpToDsp = { activeTab = MobileTab.Dsp },
          onJumpToEq = { activeTab = MobileTab.MixEq },
        )

        MobileTab.Playlists -> PlaylistsScreen(
          state = state,
          onOpenAudioFiles = openAudioFiles,
          onPlayTrack = { pid, idx -> vm.loadTrack(pid, idx, true) },
          onCreatePlaylist = { name, copy, seed -> vm.createPlaylist(name, copy, seed) },
          onSwitchPlaylist = { vm.switchPlaylist(it) },
          onRenamePlaylist = { id, name -> vm.renamePlaylist(id, name) },
          onDuplicatePlaylist = { vm.duplicatePlaylist(it) },
          onDeletePlaylist = { vm.deletePlaylist(it) },
          onAddTrackToPlaylist = { tid, idx -> vm.addTrackToPlaylist(tid, idx) },
          onMoveTrack = { from, to -> vm.moveTrack(from, to) },
          onRemoveTrack = { vm.removeTrack(it) },
          onClearActive = { vm.clearActivePlaylist() },
          onRestoreReferences = { vm.restoreStudioReferences() },
        )

        MobileTab.Lyrics -> LyricsScreen(
          state = state,
          fetchingOnline = fetchingLyrics,
          onUploadLyricsFile = openLyricsFile,
          onFetchOnlineLyrics = { vm.fetchOnlineLyrics() },
          onSaveLyricsText = { vm.saveLyricsText(it) },
          onSetOffsetMs = { vm.setLyricsOffsetMs(it) },
          onSeek = { vm.seekTo(it) },
          onUpdateFontSize = { sz -> vm.updateConfig { it.copy(lyricsFontSize = sz) } },
          onToggleAutoScroll = { vm.updateConfig { it.copy(lyricsAutoScroll = !it.lyricsAutoScroll) } },
        )

        MobileTab.Dsp -> DspScreen(
          state = state,
          onUpdateConfig = { vm.updateConfig(it) },
          onReanalyzeAutoVolume = { vm.reanalyzeAutoVolume() },
        )

        MobileTab.MixEq -> MixEqScreen(
          state = state,
          visualizer = visualizer,
          onUpdateConfig = { vm.updateConfig(it) },
          onToggleMuteChannel = { vm.toggleChannelMute(it) },
          onToggleSoloChannel = { vm.toggleChannelSolo(it) },
          onResetMuteSolo = { vm.resetMuteSolo() },
        )
      }

      // Subtle Swiss Toast Banner
      toast?.let { msg ->
        Box(
          modifier = Modifier
            .align(Alignment.BottomCenter)
            .padding(horizontal = 16.dp, vertical = 12.dp)
            .clip(RoundedCornerShape(4.dp))
            .background(SurfaceAlt)
            .border(1.dp, Signal, RoundedCornerShape(4.dp))
            .padding(horizontal = 14.dp, vertical = 8.dp),
        ) {
          Text(
            text = msg,
            fontFamily = TechnicalMono,
            fontSize = 10.5.sp,
            fontWeight = FontWeight.Bold,
            color = Ink,
          )
        }
      }
    }
  }
}
