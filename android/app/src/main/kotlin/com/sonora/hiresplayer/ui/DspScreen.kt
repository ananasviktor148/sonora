package com.sonora.hiresplayer.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sonora.hiresplayer.ConfigUi
import com.sonora.hiresplayer.SonoraState
import com.sonora.hiresplayer.ui.theme.*
import kotlin.math.abs

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DspScreen(
  state: SonoraState,
  onUpdateConfig: ((ConfigUi) -> ConfigUi) -> Unit,
  onReanalyzeAutoVolume: () -> Unit,
) {
  val scroll = rememberScrollState()
  val cfg = state.config

  Column(
    modifier = Modifier
      .fillMaxSize()
      .verticalScroll(scroll)
      .padding(horizontal = 16.dp, vertical = 14.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
  ) {
    // ── 01 / 64-Bit Sinc Upsampler ──
    SwissModuleCard(index = "01", title = "HI-RES SINC UPSAMPLER · 64-BIT INTERNAL") {
      Text(
        text = "Select integer oversampling ratio or fixed target rate (up to 192 kHz on Android AudioTrack). Changes apply seamlessly at the current playhead.",
        fontSize = 12.sp,
        color = InkDim,
      )
      Spacer(Modifier.height(12.dp))

      SwissKicker("MODE")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          "Bypass" to "BYPASS",
          "Factor2x" to "2× INT",
          "Factor4x" to "4× INT",
          "Factor8x" to "8× INT",
          "Fixed" to "FIXED RATE",
        ).forEach { (id, lbl) ->
          SwissToggleChip(
            label = lbl,
            selected = cfg.upsampleMode == id,
            accent = if (id == "Fixed") Brass else Signal,
            onClick = { onUpdateConfig { it.copy(upsampleMode = id) } },
          )
        }
      }

      if (cfg.upsampleMode == "Fixed") {
        Spacer(Modifier.height(10.dp))
        SwissKicker("TARGET RATE")
        Spacer(Modifier.height(6.dp))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
          listOf(
            44_100 to "44.1 kHz",
            48_000 to "48 kHz",
            88_200 to "88.2 kHz",
            96_000 to "96 kHz",
            176_400 to "176.4 kHz",
            192_000 to "192 kHz",
          ).forEach { (rate, lbl) ->
            SwissToggleChip(
              label = lbl,
              selected = cfg.upsampleFixedRate == rate,
              accent = Brass,
              onClick = { onUpdateConfig { it.copy(upsampleFixedRate = rate) } },
            )
          }
        }
      }

      Spacer(Modifier.height(12.dp))
      SwissKicker("FILTER QUALITY")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          "Fast" to "FAST (64-TAP)",
          "High" to "HIGH (128-TAP)",
          "Audiophile" to "AUDIOPHILE (256)",
          "Reference" to "REFERENCE (512)",
        ).forEach { (id, lbl) ->
          SwissToggleChip(
            label = lbl,
            selected = cfg.resamplerQuality == id,
            accent = Signal,
            onClick = { onUpdateConfig { it.copy(resamplerQuality = id) } },
          )
        }
      }

      Spacer(Modifier.height(12.dp))
      SwissKicker("FILTER ROLL-OFF")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          "Sharp" to "SHARP (0.95)",
          "Balanced" to "BALANCED (0.91)",
          "Gentle" to "GENTLE (0.85)",
        ).forEach { (id, lbl) ->
          SwissToggleChip(
            label = lbl,
            selected = cfg.filterRolloff == id,
            accent = Brass,
            onClick = { onUpdateConfig { it.copy(filterRolloff = id) } },
          )
        }
      }

      Spacer(Modifier.height(12.dp))
      Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        SwissKicker("DIGITAL HEADROOM")
        Text(String.format("%.1f dB", cfg.headroomDb), fontFamily = TechnicalMono, fontSize = 11.sp, color = Ink)
      }
      SwissSlider(
        value = cfg.headroomDb,
        onValueChange = { v -> onUpdateConfig { it.copy(headroomDb = v) } },
        valueRange = -6.0f..0.0f,
      )
      Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
          checked = cfg.softClip,
          onCheckedChange = { c -> onUpdateConfig { it.copy(softClip = c) } },
          colors = CheckboxDefaults.colors(checkedColor = Signal),
        )
        Text("Soft-Clip Peak Limiter", fontSize = 12.5.sp, color = InkDim)
      }
    }

    // ── 02 / Native DSD Engine ──
    SwissModuleCard(index = "02", title = "NATIVE DSD ENGINE · DSF / DFF · DSD64–512") {
      Text(
        text = "Byte-LUT linear-phase Blackman-Harris FIR polyphase decimator converting 1-bit DSD directly into 32-bit float PCM.",
        fontSize = 12.sp,
        color = InkDim,
      )
      Spacer(Modifier.height(12.dp))

      SwissKicker("PCM DECIMATION RATE")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          "Pcm88k" to "88.2 kHz",
          "Pcm176k" to "176.4 kHz (AUDIOPHILE)",
          "Pcm352k" to "352.8 kHz (DXD)",
        ).forEach { (id, lbl) ->
          SwissToggleChip(
            label = lbl,
            selected = cfg.dsdOutputMode == id,
            accent = Brass,
            onClick = { onUpdateConfig { it.copy(dsdOutputMode = id) } },
          )
        }
      }

      Spacer(Modifier.height(12.dp))
      SwissKicker("ULTRASONIC LOW-PASS FILTER")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(24_000, 30_000, 40_000, 50_000).forEach { hz ->
          SwissToggleChip(
            label = "${hz / 1000} kHz",
            selected = cfg.dsdCutoffHz == hz,
            accent = Signal,
            onClick = { onUpdateConfig { it.copy(dsdCutoffHz = hz) } },
          )
        }
      }

      Spacer(Modifier.height(8.dp))
      Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
          checked = cfg.dsdGainBoost6db,
          onCheckedChange = { c -> onUpdateConfig { it.copy(dsdGainBoost6db = c) } },
          colors = CheckboxDefaults.colors(checkedColor = Brass),
        )
        Text("+6 dB SACD Gain Compensation", fontSize = 12.5.sp, color = InkDim)
      }
    }

    // ── 03 / Auto Adjust Playback Volume (KORG AudioGate Engine) ──
    SwissModuleCard(index = "03", title = "AUTO ADJUST PLAYBACK VOLUME · AUDIOGATE") {
      Text(
        text = "Non-destructive real-time loudness normalization across PCM and DSD tracks (ITU-R BS.1770-4 / EBU R128 + ReplayGain tags). Song mode levels each track independently; Album mode preserves relative track dynamics within an album.",
        fontSize = 12.sp,
        color = InkDim,
      )
      Spacer(Modifier.height(12.dp))

      SwissKicker("MODE")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          Triple("Off", "OFF", Ink),
          Triple("Song", "SONG (PER TRACK)", Mint),
          Triple("Album", "ALBUM (PRESERVE DYNAMICS)", Brass),
        ).forEach { (id, lbl, accent) ->
          SwissToggleChip(
            label = lbl,
            selected = cfg.autoVolMode == id,
            accent = accent,
            onClick = { onUpdateConfig { it.copy(autoVolMode = id) } },
          )
        }
      }

      Spacer(Modifier.height(12.dp))
      Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        SwissKicker("TARGET LOUDNESS")
        Text(String.format("%.1f LUFS", cfg.autoVolTargetLufs), fontFamily = TechnicalMono, fontSize = 11.sp, color = Signal)
      }
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          -23.0f to "-23 EBU R128",
          -18.0f to "-18 REPLAYGAIN",
          -16.0f to "-16 AUDIOPHILE",
          -14.0f to "-14 STREAMING",
        ).forEach { (lufs, lbl) ->
          SwissToggleChip(
            label = lbl,
            selected = abs(cfg.autoVolTargetLufs - lufs) < 0.15f,
            accent = Signal,
            onClick = { onUpdateConfig { it.copy(autoVolTargetLufs = lufs) } },
          )
        }
      }
      SwissSlider(
        value = cfg.autoVolTargetLufs,
        onValueChange = { v -> onUpdateConfig { it.copy(autoVolTargetLufs = v) } },
        valueRange = -24.0f..-10.0f,
      )

      Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
          checked = cfg.autoVolPreventClipping,
          onCheckedChange = { c -> onUpdateConfig { it.copy(autoVolPreventClipping = c) } },
          colors = CheckboxDefaults.colors(checkedColor = Mint),
        )
        Text("Prevent Clipping (-1.0 dBFS Peak Ceiling)", fontSize = 12.5.sp, color = InkDim)
      }
      Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
          checked = cfg.autoVolPreferRgTags,
          onCheckedChange = { c -> onUpdateConfig { it.copy(autoVolPreferRgTags = c) } },
          colors = CheckboxDefaults.colors(checkedColor = Brass),
        )
        Text("Prefer Embedded ReplayGain / R128 Tags", fontSize = 12.5.sp, color = InkDim)
      }

      Spacer(Modifier.height(10.dp))

      // Live Loudness Telemetry Readout
      val av = state.autoVolume
      SwissSubGroup(fillColor = Ground) {
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
          SwissKicker("LIVE LOUDNESS TELEMETRY", color = Brass, modifier = Modifier.weight(1f))
          if (cfg.autoVolMode != "Off" && state.trackInfo != null) {
            SwissGhostButton("↻ RE-ANALYZE", onClick = onReanalyzeAutoVolume)
          }
        }
        Spacer(Modifier.height(8.dp))

        val (statusTxt, statusCol) = when {
          cfg.autoVolMode == "Off" -> "OFF · ORIGINAL MASTER LEVEL" to InkMute
          av.analyzing -> "ANALYZING ${cfg.autoVolMode.uppercase()} LOUDNESS…" to Signal
          av.active -> "ACTIVE · ${cfg.autoVolMode.uppercase()} MODE" to Mint
          else -> "READY · LOAD A TRACK" to InkMute
        }
        val measTxt = when {
          av.measuredLufs != null && av.peakDbfs != null ->
            String.format("%.1f LUFS · PEAK %.1f dBFS", av.measuredLufs, av.peakDbfs)
          av.measuredLufs != null -> String.format("%.1f LUFS", av.measuredLufs)
          else -> "—"
        }
        val (gainTxt, gainCol) = when {
          cfg.autoVolMode == "Off" || !av.active -> "0.0 dB" to InkMute
          av.peakLimited -> String.format("%+.1f dB (PEAK SAFE)", av.appliedGainDb) to Signal
          av.appliedGainDb > 0.05f -> String.format("%+.1f dB", av.appliedGainDb) to Brass
          else -> String.format("%+.1f dB", av.appliedGainDb) to Mint
        }

        TelemetryRow("STATUS", statusTxt, statusCol, bold = true)
        TelemetryRow("MEASURED", measTxt, Ink)
        TelemetryRow("APPLIED GAIN", gainTxt, gainCol, bold = true)
        TelemetryRow("SOURCE", av.sourceLabel, InkDim)
      }
    }
  }
}

@Composable
private fun TelemetryRow(
  label: String,
  value: String,
  color: androidx.compose.ui.graphics.Color,
  bold: Boolean = false,
) {
  Row(
    modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Text(
      text = label,
      fontFamily = TechnicalMono,
      fontSize = 9.5.sp,
      color = InkMute,
      modifier = Modifier.width(100.dp),
    )
    Text(
      text = value,
      fontFamily = TechnicalMono,
      fontSize = 10.5.sp,
      fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal,
      color = color,
    )
  }
}
