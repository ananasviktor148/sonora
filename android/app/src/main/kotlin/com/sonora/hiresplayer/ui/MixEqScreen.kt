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

private val EQ_BAND_LABELS = listOf("31 Hz", "62 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz")

private val EQ_PRESETS: List<Triple<String, Float, List<Float>>> = listOf(
  Triple("Flat", 0.0f, List(10) { 0.0f }),
  Triple("Audiophile Warm", -1.0f, listOf(1.5f, 2.0f, 1.5f, 0.5f, 0.0f, -0.5f, 0.0f, 1.0f, 1.5f, 1.0f)),
  Triple("Studio Reference", 0.0f, listOf(0.5f, 0.0f, 0.0f, -0.5f, 0.0f, 0.0f, 0.5f, 0.5f, 1.0f, 0.5f)),
  Triple("Vocal Clarity", -1.5f, listOf(-1.0f, -0.5f, 0.0f, 1.5f, 3.0f, 3.5f, 2.5f, 1.0f, 0.5f, 0.0f)),
  Triple("Deep Bass", -3.0f, listOf(5.0f, 4.5f, 3.0f, 1.0f, 0.0f, 0.0f, 0.5f, 1.0f, 1.0f, 0.5f)),
  Triple("Classical Hall", -1.0f, listOf(1.0f, 0.5f, 0.0f, 0.0f, -0.5f, -0.5f, 0.5f, 1.5f, 2.5f, 2.0f)),
  Triple("Jazz Club", -1.0f, listOf(2.0f, 1.5f, 1.0f, 1.5f, -0.5f, -0.5f, 0.5f, 1.5f, 2.0f, 1.5f)),
  Triple("Electronic", -2.5f, listOf(4.0f, 3.5f, 1.5f, 0.0f, -1.0f, 0.5f, 1.5f, 2.5f, 3.5f, 3.0f)),
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun MixEqScreen(
  state: SonoraState,
  visualizer: FloatArray,
  onUpdateConfig: ((ConfigUi) -> ConfigUi) -> Unit,
  onToggleMuteChannel: (Int) -> Unit,
  onToggleSoloChannel: (Int) -> Unit,
  onResetMuteSolo: () -> Unit,
) {
  val scroll = rememberScrollState()
  val cfg = state.config
  val info = state.trackInfo

  Column(
    modifier = Modifier
      .fillMaxSize()
      .verticalScroll(scroll)
      .padding(horizontal = 16.dp, vertical = 14.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
  ) {
    // ── 01 / Multi-Channel Matrix & Per-Channel Mute/Solo ──
    SwissModuleCard(
      index = "01",
      title = "MULTI-CHANNEL MATRIX · 1.0 – 7.1",
      headerAction = {
        SwissGhostButton("RESET M/S", onClick = onResetMuteSolo)
      },
    ) {
      SwissKicker("ROUTING & DOWNMIX MODE")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        listOf(
          "Auto" to "AUTO",
          "NativePassthrough" to "NATIVE",
          "StereoItu" to "ITU-R BS.775 STEREO",
          "HeadphoneSpatial" to "HEADPHONE SPATIAL",
        ).forEach { (id, lbl) ->
          SwissToggleChip(
            label = lbl,
            selected = cfg.downmixMode == id,
            accent = Signal,
            onClick = { onUpdateConfig { it.copy(downmixMode = id) } },
          )
        }
      }

      Spacer(Modifier.height(10.dp))
      LabeledSliderRow("CENTER GAIN", cfg.centerGainDb, -12f..6f, " dB") { v ->
        onUpdateConfig { it.copy(centerGainDb = v) }
      }
      LabeledSliderRow("SURROUND GAIN", cfg.surroundGainDb, -12f..6f, " dB") { v ->
        onUpdateConfig { it.copy(surroundGainDb = v) }
      }
      Row(verticalAlignment = Alignment.CenterVertically) {
        Checkbox(
          checked = cfg.includeLfe,
          onCheckedChange = { c -> onUpdateConfig { it.copy(includeLfe = c) } },
          colors = CheckboxDefaults.colors(checkedColor = Brass),
        )
        Text("Include LFE (.1 Subwoofer) in Downmix", fontSize = 12.sp, color = InkDim)
      }
      if (cfg.includeLfe) {
        LabeledSliderRow("LFE GAIN", cfg.lfeGainDb, -15f..3f, " dB") { v ->
          onUpdateConfig { it.copy(lfeGainDb = v) }
        }
      }
      if (cfg.downmixMode == "HeadphoneSpatial") {
        LabeledSliderRow("SPATIAL CROSSFEED", cfg.crossfeedAmount * 100f, 0f..50f, "%") { v ->
          onUpdateConfig { it.copy(crossfeedAmount = v / 100f) }
        }
      }

      Spacer(Modifier.height(10.dp))
      SwissSubGroup(fillColor = Ground) {
        val chCount = (info?.channels ?: 2).coerceIn(1, 8)
        SwissKicker("CHANNEL CONSOLE (${info?.channelLayout?.uppercase() ?: "STEREO"})", color = Brass)
        Spacer(Modifier.height(8.dp))
        for (ch in 0 until chCount) {
          val lbl = info?.channelLabels?.getOrElse(ch) { "CH${ch + 1}" } ?: "CH${ch + 1}"
          MiniChannelClickableRow(
            label = lbl,
            peak = visualizer.getOrElse(96 + ch) { 0f },
            rms = visualizer.getOrElse(104 + ch) { 0f },
            hold = visualizer.getOrElse(112 + ch) { 0f },
            muted = state.channelMuted.getOrElse(ch) { false },
            solo = state.channelSolo.getOrElse(ch) { false },
            onToggleMute = { onToggleMuteChannel(ch) },
            onToggleSolo = { onToggleSoloChannel(ch) },
          )
        }
      }
    }

    // ── 02 / 10-Band 64-Bit Parametric Equalizer ──
    SwissModuleCard(
      index = "02",
      title = "10-BAND 64-BIT EQUALIZER",
      headerAction = {
        SwissToggleChip(
          label = if (cfg.eqEnabled) "ACTIVE ●" else "BYPASS ○",
          selected = cfg.eqEnabled,
          accent = Brass,
          onClick = { onUpdateConfig { it.copy(eqEnabled = !it.eqEnabled) } },
        )
      },
    ) {
      SwissKicker("PRESETS")
      Spacer(Modifier.height(6.dp))
      FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        EQ_PRESETS.forEach { (name, preamp, bands) ->
          SwissToggleChip(
            label = name.uppercase(),
            selected = cfg.eqPresetName == name,
            accent = Brass,
            onClick = {
              onUpdateConfig {
                it.copy(
                  eqEnabled = name != "Flat",
                  eqPreampDb = preamp,
                  eqBandsDb = bands,
                  eqPresetName = name,
                )
              }
            },
          )
        }
      }

      Spacer(Modifier.height(12.dp))
      SwissSubGroup(fillColor = Ground) {
        // Preamp slider
        LabeledSliderRow("PREAMP", cfg.eqPreampDb, -12f..12f, " dB", accent = Brass) { v ->
          onUpdateConfig { it.copy(eqPreampDb = v, eqPresetName = "Custom") }
        }
        Spacer(Modifier.height(6.dp))
        SwissHairline()
        Spacer(Modifier.height(6.dp))

        for (i in 0 until 10) {
          val valDb = cfg.eqBandsDb.getOrElse(i) { 0f }
          Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
          ) {
            Text(
              text = EQ_BAND_LABELS[i],
              fontFamily = TechnicalMono,
              fontSize = 10.sp,
              color = InkDim,
              modifier = Modifier.width(50.dp),
            )
            SwissSlider(
              value = valDb,
              onValueChange = { v ->
                val updatedBands = cfg.eqBandsDb.toMutableList().apply {
                  while (size < 10) add(0f)
                  this[i] = v
                }
                onUpdateConfig {
                  it.copy(
                    eqEnabled = true,
                    eqBandsDb = updatedBands,
                    eqPresetName = "Custom",
                  )
                }
              },
              valueRange = -12f..12f,
              accent = if (abs(valDb) > 0.1f) Signal else InkDim,
              modifier = Modifier.weight(1f),
            )
            Text(
              text = String.format("%+5.1f", valDb),
              fontFamily = TechnicalMono,
              fontSize = 10.sp,
              fontWeight = if (abs(valDb) > 0.1f) FontWeight.Bold else FontWeight.Normal,
              color = if (abs(valDb) > 0.1f) Signal else InkMute,
              modifier = Modifier.width(42.dp),
            )
          }
        }
      }
    }
  }
}

@Composable
private fun LabeledSliderRow(
  label: String,
  value: Float,
  range: ClosedFloatingPointRange<Float>,
  suffix: String,
  accent: androidx.compose.ui.graphics.Color = Signal,
  onChange: (Float) -> Unit,
) {
  Column(modifier = Modifier.fillMaxWidth()) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
      SwissKicker(label)
      Text(String.format("%+.1f%s", value, suffix), fontFamily = TechnicalMono, fontSize = 10.5.sp, color = Ink)
    }
    SwissSlider(value = value, onValueChange = onChange, valueRange = range, accent = accent)
  }
}
