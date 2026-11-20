package com.sonora.hiresplayer.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sonora.hiresplayer.*
import com.sonora.hiresplayer.ui.theme.*
import kotlin.math.log10

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun NowPlayingScreen(
  state: SonoraState,
  visualizer: FloatArray,
  coverBitmap: ImageBitmap?,
  onTogglePlayPause: () -> Unit,
  onStop: () -> Unit,
  onPrev: () -> Unit,
  onNext: () -> Unit,
  onSeek: (Double) -> Unit,
  onToggleShuffle: () -> Unit,
  onCycleRepeat: () -> Unit,
  onSetVolume: (Float) -> Unit,
  onToggleMute: () -> Unit,
  onCycleAutoVolume: () -> Unit,
  onOpenFiles: () -> Unit,
  onRestoreReferences: () -> Unit,
  onJumpToDsp: () -> Unit,
  onJumpToEq: () -> Unit,
) {
  val scroll = rememberScrollState()
  val info = state.trackInfo

  Column(
    modifier = Modifier
      .fillMaxSize()
      .verticalScroll(scroll)
      .padding(horizontal = 16.dp, vertical = 14.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
  ) {
    if (info == null) {
      SwissModuleCard(index = "00", title = "SONORA HI-RES MASTERING CONSOLE") {
        Text(
          text = "Open local audio files or restore the studio reference catalog.",
          fontFamily = EditorialSerif,
          fontSize = 22.sp,
          lineHeight = 28.sp,
          color = Ink,
        )
        Spacer(Modifier.height(8.dp))
        Text(
          text = "SYMPHONIA NATIVE PCM · NATIVE DSD64–512 (DSF/DFF) · 64-BIT SINC UPSAMPLER · AUDIOGATE AUTO VOLUME",
          fontFamily = TechnicalMono,
          fontSize = 10.sp,
          color = Mint,
        )
        Spacer(Modifier.height(16.dp))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
          SwissPrimaryButton("+ ADD MUSIC FILES", onClick = onOpenFiles)
          SwissSecondaryButton("RESTORE STUDIO REFERENCES", onClick = onRestoreReferences, accent = Brass)
        }
      }
      return
    }

    // ── Hero Album Art + Track Metadata Card ──
    Column(
      modifier = Modifier
        .fillMaxWidth()
        .clip(RoundedCornerShape(6.dp))
        .background(Surface)
        .border(1.dp, Rule, RoundedCornerShape(6.dp))
        .padding(16.dp),
    ) {
      // Top Album Artwork Plate centered
      Box(
        modifier = Modifier
          .fillMaxWidth()
          .wrapContentWidth(Alignment.CenterHorizontally)
          .size(216.dp)
          .clip(RoundedCornerShape(5.dp))
          .background(Well)
          .border(1.dp, RuleStrong, RoundedCornerShape(5.dp)),
        contentAlignment = Alignment.Center,
      ) {
        if (coverBitmap != null) {
          Image(
            bitmap = coverBitmap,
            contentDescription = info.title,
            contentScale = ContentScale.Crop,
            modifier = Modifier.fillMaxSize(),
          )
        } else {
          Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(
              text = info.format.ifBlank { "PCM" },
              fontFamily = TechnicalMono,
              fontSize = 20.sp,
              fontWeight = FontWeight.Bold,
              color = Brass,
            )
            Spacer(Modifier.height(4.dp))
            Text(
              text = formatRate(info.sampleRate),
              fontFamily = TechnicalMono,
              fontSize = 11.sp,
              color = InkMute,
            )
          }
        }
      }

      Spacer(Modifier.height(14.dp))

      // Certification Stamps
      FlowRow(
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
      ) {
        if (info.isDsd) {
          SwissStampTag(info.codec, Brass, Brass)
        } else if (info.isHires) {
          SwissStampTag("HI-RES MASTER", Brass, Brass)
        }
        if (state.upsampled) {
          SwissStampTag("↑ ${formatRate(state.outputSampleRate)}", Signal, Signal)
        }
        if (info.channels > 2) {
          SwissStampTag(info.channelLayout, Ink, RuleStrong)
        }
        if (state.downmixed) {
          SwissStampTag("${state.sourceChannels}→${state.outputChannels}CH MATRIX", InkDim, Rule)
        }
        if (state.signalPath.bitPerfect) {
          SwissStampTag("● BIT-PERFECT", Mint, Mint)
        }
        if (info.isDemo) {
          SwissStampTag("STUDIO REF", Brass, RuleStrong)
        }
      }

      Spacer(Modifier.height(12.dp))

      // Editorial Serif Track Title / Artist / Album
      Text(
        text = info.title,
        fontFamily = EditorialSerif,
        fontSize = 24.sp,
        lineHeight = 30.sp,
        color = Ink,
      )
      if (info.artist.isNotBlank()) {
        Spacer(Modifier.height(3.dp))
        Text(
          text = info.artist,
          fontFamily = EditorialSerif,
          fontSize = 16.sp,
          color = InkDim,
        )
      }
      if (info.album.isNotBlank()) {
        Spacer(Modifier.height(4.dp))
        Text(
          text = info.album.uppercase(),
          fontFamily = TechnicalMono,
          fontSize = 10.sp,
          color = InkMute,
        )
      }

      Spacer(Modifier.height(14.dp))

      // 2x2 Specification Grid
      val rateText = if (state.outputSampleRate > 0 && state.outputSampleRate != info.sampleRate) {
        "${formatRate(info.sampleRate)} → ${formatRate(state.outputSampleRate)}"
      } else formatRate(info.sampleRate)
      val bitText = if (info.isDsd) "1-BIT DSD" else "${info.bitDepth}-BIT PCM"
      val chText = if (state.outputChannels > 0 && state.outputChannels != info.channels) {
        "${info.channels} → ${state.outputChannels}CH"
      } else info.channelLayout.uppercase()

      SwissSubGroup(fillColor = Ground) {
        Row(modifier = Modifier.fillMaxWidth()) {
          SpecCell("FORMAT", "${info.format} · ${info.codec}", Ink, Modifier.weight(1f))
          Spacer(Modifier.width(12.dp))
          SpecCell("SAMPLE RATE", rateText, if (state.upsampled) Signal else Ink, Modifier.weight(1f))
        }
        Spacer(Modifier.height(10.dp))
        Row(modifier = Modifier.fillMaxWidth()) {
          SpecCell("DEPTH", bitText, if (info.isDsd) Brass else Ink, Modifier.weight(1f))
          Spacer(Modifier.width(12.dp))
          SpecCell("CHANNELS", chText, Ink, Modifier.weight(1f))
        }
      }

      Spacer(Modifier.height(14.dp))

      // Precision Scrub Rail
      SwissScrubBar(
        position = state.position,
        duration = state.duration,
        onSeek = onSeek,
      )
      Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
      ) {
        Text(formatTime(state.position), fontFamily = TechnicalMono, fontSize = 10.5.sp, color = Ink)
        Text(formatTime(state.duration), fontFamily = TechnicalMono, fontSize = 10.5.sp, color = InkMute)
      }

      Spacer(Modifier.height(12.dp))

      // Hardware Transport Row
      Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
      ) {
        SwissGhostButton(
          label = "SHUF",
          onClick = onToggleShuffle,
          accent = if (state.config.shuffle) Signal else null,
        )
        SwissSecondaryButton("⏮", onClick = onPrev)
        SwissPrimaryButton(
          label = if (state.isPlaying) "⏸ PAUSE" else "▶ PLAY",
          onClick = onTogglePlayPause,
          fillColor = if (state.isPlaying) Ink else Signal,
          textColor = Ground,
          modifier = Modifier.width(104.dp).height(40.dp),
        )
        SwissSecondaryButton("⏭", onClick = onNext)
        val (repLbl, repAccent) = when (state.config.repeatMode) {
          "All" -> "RPT ALL" to Signal
          "One" -> "RPT 1" to Brass
          else -> "RPT" to null
        }
        SwissGhostButton(label = repLbl, onClick = onCycleRepeat, accent = repAccent)
      }

      Spacer(Modifier.height(12.dp))
      SwissHairline()
      Spacer(Modifier.height(10.dp))

      // Volume + 1-Tap AudioGate Auto Adjust Volume Row
      Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        SwissGhostButton(
          label = if (state.muted || state.config.volume <= 0.001f) "MUTE" else "VOL",
          onClick = onToggleMute,
          accent = if (state.muted) Alert else null,
        )
        Spacer(Modifier.width(8.dp))
        SwissSlider(
          value = state.config.volume,
          onValueChange = onSetVolume,
          valueRange = 0f..1f,
          modifier = Modifier.weight(1f),
        )
        Spacer(Modifier.width(8.dp))
        val volText = if (state.muted || state.config.volume <= 0.001f) {
          "-∞ dB"
        } else {
          String.format("%+.1f dB", 20f * log10(state.config.volume))
        }
        Text(volText, fontFamily = TechnicalMono, fontSize = 10.sp, color = InkDim)
      }

      Spacer(Modifier.height(8.dp))

      // Quick DSP & AudioGate Auto Volume Row
      FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
      ) {
        val avMode = state.config.autoVolMode
        val avAccent = when (avMode) {
          "Song" -> Mint
          "Album" -> Brass
          else -> null
        }
        val avText = when (avMode) {
          "Off" -> "AUTO VOL · OFF"
          else -> {
            val av = state.autoVolume
            when {
              av.analyzing -> "AUTO · ${avMode.uppercase()} (SCAN)"
              av.active -> String.format("AUTO · %s (%+.1f dB)", avMode.uppercase(), av.appliedGainDb)
              else -> "AUTO · ${avMode.uppercase()}"
            }
          }
        }
        SwissSecondaryButton(avText, onClick = onCycleAutoVolume, accent = avAccent)

        val upLbl = when (state.config.upsampleMode) {
          "Bypass" -> "UPSAMPLER · BYPASS"
          "Fixed" -> "UPSAMPLER · ${formatRate(state.config.upsampleFixedRate)}"
          else -> "UPSAMPLER · ${state.config.upsampleMode.removePrefix("Factor").uppercase()}"
        }
        SwissSecondaryButton(
          label = upLbl,
          onClick = onJumpToDsp,
          accent = if (state.config.upsampleMode != "Bypass") Signal else null,
        )

        SwissSecondaryButton(
          label = if (state.config.eqEnabled) "EQ · ${state.config.eqPresetName.uppercase()}" else "EQ · BYPASS",
          onClick = onJumpToEq,
          accent = if (state.config.eqEnabled) Brass else null,
        )

        SwissGhostButton("⏹ STOP", onClick = onStop)
      }
    }

    // ── 01 / Real-Time 48-Band FFT Spectrum Analyzer ──
    SwissModuleCard(index = "01", title = "REAL-TIME SPECTRUM · 2048-PT FFT") {
      SpectrumAnalyzerCanvas(visualizer = visualizer)
    }

    // ── 02 / Multi-Channel Peak / RMS Meter Bridge ──
    val chCount = info.channels.coerceIn(1, 8)
    SwissModuleCard(index = "02", title = "CHANNEL METERS · ${info.channelLayout.uppercase()}") {
      Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        for (ch in 0 until chCount) {
          val lbl = info.channelLabels.getOrElse(ch) { "CH${ch + 1}" }
          ChannelMeterBar(
            label = lbl,
            peak = visualizer.getOrElse(96 + ch) { 0f },
            rms = visualizer.getOrElse(104 + ch) { 0f },
            hold = visualizer.getOrElse(112 + ch) { 0f },
          )
        }
      }
    }

    // ── 03 / Audio Signal Chain ──
    if (state.signalPath.stages.isNotEmpty()) {
      SwissModuleCard(index = "03", title = "AUDIO SIGNAL CHAIN") {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
          state.signalPath.stages.forEach { stage ->
            Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
              Text(
                text = "■",
                fontSize = 8.sp,
                color = if (stage.lossless) Mint else Brass,
                modifier = Modifier.padding(top = 3.dp, end = 8.dp),
              )
              Column(modifier = Modifier.weight(1f)) {
                Text(
                  text = stage.title.uppercase(),
                  fontFamily = TechnicalMono,
                  fontSize = 10.sp,
                  fontWeight = FontWeight.Bold,
                  color = Ink,
                )
                Text(
                  text = stage.detail,
                  fontFamily = TechnicalMono,
                  fontSize = 10.5.sp,
                  color = InkDim,
                )
              }
            }
          }
        }
      }
    }

    state.errorMessage?.let { err ->
      SwissSubGroup(borderColor = Alert, fillColor = Surface) {
        Text(err, fontFamily = TechnicalMono, fontSize = 11.sp, color = Alert)
      }
    }
  }
}

@Composable
private fun SpecCell(label: String, value: String, valueColor: androidx.compose.ui.graphics.Color, modifier: Modifier) {
  Column(modifier = modifier) {
    Text(label, fontFamily = TechnicalMono, fontSize = 9.sp, color = InkMute)
    Spacer(Modifier.height(2.dp))
    Text(
      text = value,
      fontFamily = TechnicalMono,
      fontSize = 11.sp,
      fontWeight = FontWeight.Bold,
      color = valueColor,
      maxLines = 1,
      overflow = TextOverflow.Ellipsis,
    )
  }
}
