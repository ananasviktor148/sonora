package com.sonora.hiresplayer.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sonora.hiresplayer.ui.theme.*
import kotlin.math.log10

@Composable
fun SwissHairline(color: Color = Rule, modifier: Modifier = Modifier) {
  Box(
    modifier = modifier
      .fillMaxWidth()
      .height(1.dp)
      .background(color),
  )
}

@Composable
fun SwissKicker(text: String, color: Color = InkMute, modifier: Modifier = Modifier) {
  Text(
    text = text.uppercase(),
    fontFamily = TechnicalMono,
    fontSize = 10.sp,
    fontWeight = FontWeight.Bold,
    letterSpacing = 0.8.sp,
    color = color,
    modifier = modifier,
  )
}

@Composable
fun SwissModuleCard(
  index: String,
  title: String,
  modifier: Modifier = Modifier,
  headerAction: (@Composable () -> Unit)? = null,
  content: @Composable ColumnScope.() -> Unit,
) {
  Column(
    modifier = modifier
      .fillMaxWidth()
      .clip(RoundedCornerShape(6.dp))
      .background(Surface)
      .border(1.dp, Rule, RoundedCornerShape(6.dp))
      .padding(16.dp),
  ) {
    Row(
      modifier = Modifier.fillMaxWidth(),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      Text(
        text = index,
        fontFamily = TechnicalMono,
        fontSize = 10.sp,
        fontWeight = FontWeight.Bold,
        color = Signal,
      )
      Spacer(Modifier.width(8.dp))
      Text(
        text = title.uppercase(),
        fontFamily = TechnicalMono,
        fontSize = 10.sp,
        fontWeight = FontWeight.Bold,
        letterSpacing = 0.6.sp,
        color = InkDim,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier.weight(1f),
      )
      headerAction?.invoke()
    }
    Spacer(Modifier.height(10.dp))
    SwissHairline()
    Spacer(Modifier.height(12.dp))
    content()
  }
}

@Composable
fun SwissSubGroup(
  modifier: Modifier = Modifier,
  borderColor: Color = Rule,
  fillColor: Color = SurfaceAlt,
  content: @Composable ColumnScope.() -> Unit,
) {
  Column(
    modifier = modifier
      .fillMaxWidth()
      .clip(RoundedCornerShape(5.dp))
      .background(fillColor)
      .border(1.dp, borderColor, RoundedCornerShape(5.dp))
      .padding(12.dp),
    content = content,
  )
}

@Composable
fun SwissPrimaryButton(
  label: String,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
  fillColor: Color = Signal,
  textColor: Color = Ground,
) {
  Surface(
    onClick = onClick,
    shape = RoundedCornerShape(4.dp),
    color = fillColor,
    border = BorderStroke(1.dp, fillColor),
    modifier = modifier.heightIn(min = 34.dp),
  ) {
    Box(
      contentAlignment = Alignment.Center,
      modifier = Modifier.padding(horizontal = 12.dp, vertical = 7.dp),
    ) {
      Text(
        text = label,
        fontFamily = TechnicalMono,
        fontSize = 11.sp,
        fontWeight = FontWeight.Bold,
        color = textColor,
        maxLines = 1,
      )
    }
  }
}

@Composable
fun SwissSecondaryButton(
  label: String,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
  accent: Color? = null,
) {
  Surface(
    onClick = onClick,
    shape = RoundedCornerShape(4.dp),
    color = SurfaceAlt,
    border = BorderStroke(1.dp, accent ?: RuleStrong),
    modifier = modifier.heightIn(min = 34.dp),
  ) {
    Box(
      contentAlignment = Alignment.Center,
      modifier = Modifier.padding(horizontal = 11.dp, vertical = 7.dp),
    ) {
      Text(
        text = label,
        fontFamily = TechnicalMono,
        fontSize = 10.5.sp,
        fontWeight = FontWeight.SemiBold,
        color = accent ?: Ink,
        maxLines = 1,
      )
    }
  }
}

@Composable
fun SwissGhostButton(
  label: String,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
  accent: Color? = null,
) {
  Surface(
    onClick = onClick,
    shape = RoundedCornerShape(4.dp),
    color = Color.Transparent,
    border = BorderStroke(1.dp, accent ?: Rule),
    modifier = modifier.heightIn(min = 30.dp),
  ) {
    Box(
      contentAlignment = Alignment.Center,
      modifier = Modifier.padding(horizontal = 9.dp, vertical = 5.dp),
    ) {
      Text(
        text = label,
        fontFamily = TechnicalMono,
        fontSize = 10.sp,
        fontWeight = FontWeight.SemiBold,
        color = accent ?: InkDim,
        maxLines = 1,
      )
    }
  }
}

@Composable
fun SwissToggleChip(
  label: String,
  selected: Boolean,
  accent: Color,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
) {
  Surface(
    onClick = onClick,
    shape = RoundedCornerShape(4.dp),
    color = if (selected) SurfaceAlt else Ground,
    border = BorderStroke(1.dp, if (selected) accent else Rule),
    modifier = modifier.heightIn(min = 32.dp),
  ) {
    Box(
      contentAlignment = Alignment.Center,
      modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
    ) {
      Text(
        text = label,
        fontFamily = TechnicalMono,
        fontSize = 10.sp,
        fontWeight = if (selected) FontWeight.Bold else FontWeight.Normal,
        color = if (selected) accent else InkDim,
        maxLines = 1,
      )
    }
  }
}

@Composable
fun SwissStampTag(text: String, textColor: Color, borderColor: Color) {
  Box(
    modifier = Modifier
      .clip(RoundedCornerShape(3.dp))
      .background(Ground)
      .border(1.dp, borderColor, RoundedCornerShape(3.dp))
      .padding(horizontal = 7.dp, vertical = 3.dp),
  ) {
    Text(
      text = text.uppercase(),
      fontFamily = TechnicalMono,
      fontSize = 9.sp,
      fontWeight = FontWeight.Bold,
      color = textColor,
    )
  }
}

@Composable
fun SwissSlider(
  value: Float,
  onValueChange: (Float) -> Unit,
  valueRange: ClosedFloatingPointRange<Float>,
  modifier: Modifier = Modifier,
  accent: Color = Signal,
  onValueChangeFinished: (() -> Unit)? = null,
) {
  Slider(
    value = value.coerceIn(valueRange.start, valueRange.endInclusive),
    onValueChange = onValueChange,
    onValueChangeFinished = onValueChangeFinished,
    valueRange = valueRange,
    colors = SliderDefaults.colors(
      thumbColor = Ink,
      activeTrackColor = accent,
      inactiveTrackColor = Rule,
    ),
    modifier = modifier,
  )
}

@Composable
fun SwissScrubBar(
  position: Double,
  duration: Double,
  onSeek: (Double) -> Unit,
  modifier: Modifier = Modifier,
) {
  var draggingFrac by remember { mutableStateOf<Float?>(null) }
  val liveFrac = if (duration > 0.0) (position / duration).toFloat().coerceIn(0f, 1f) else 0f
  val shownFrac = draggingFrac ?: liveFrac

  Box(
    modifier = modifier
      .fillMaxWidth()
      .height(28.dp)
      .pointerInput(duration) {
        detectTapGestures { offset ->
          if (duration > 0.0 && size.width > 0) {
            val f = (offset.x / size.width).coerceIn(0f, 1f)
            onSeek(f.toDouble() * duration)
          }
        }
      }
      .pointerInput(duration) {
        detectHorizontalDragGestures(
          onDragStart = { offset ->
            if (duration > 0.0 && size.width > 0) {
              draggingFrac = (offset.x / size.width).coerceIn(0f, 1f)
            }
          },
          onHorizontalDrag = { change, _ ->
            if (duration > 0.0 && size.width > 0) {
              draggingFrac = (change.position.x / size.width).coerceIn(0f, 1f)
            }
          },
          onDragEnd = {
            draggingFrac?.let { f ->
              if (duration > 0.0) onSeek(f.toDouble() * duration)
            }
            draggingFrac = null
          },
          onDragCancel = { draggingFrac = null },
        )
      },
  ) {
    Canvas(modifier = Modifier.fillMaxSize()) {
      val cy = size.height / 2f
      drawLine(Rule, Offset(0f, cy), Offset(size.width, cy), strokeWidth = 3.dp.toPx())
      val headX = size.width * shownFrac
      if (shownFrac > 0f) {
        drawLine(Signal, Offset(0f, cy), Offset(headX, cy), strokeWidth = 3.dp.toPx())
      }
      drawRoundRect(
        color = Ink,
        topLeft = Offset((headX - 3.dp.toPx()).coerceIn(0f, size.width - 6.dp.toPx()), cy - 8.dp.toPx()),
        size = Size(6.dp.toPx(), 16.dp.toPx()),
        cornerRadius = CornerRadius(2.dp.toPx()),
      )
    }
  }
}

@Composable
fun SpectrumAnalyzerCanvas(
  visualizer: FloatArray,
  modifier: Modifier = Modifier,
) {
  Column(
    modifier = modifier
      .fillMaxWidth()
      .clip(RoundedCornerShape(5.dp))
      .background(Well)
      .border(1.dp, Rule, RoundedCornerShape(5.dp))
      .padding(horizontal = 10.dp, vertical = 10.dp),
  ) {
    Canvas(
      modifier = Modifier
        .fillMaxWidth()
        .height(118.dp),
    ) {
      val bands = 48
      val gridColor = Color(0xFF141419)
      for (i in 1..3) {
        val y = size.height * (i / 4f)
        drawLine(gridColor, Offset(0f, y), Offset(size.width, y), strokeWidth = 1.dp.toPx())
      }

      val slotW = size.width / bands
      val barW = (slotW * 0.64f).coerceAtLeast(2f)
      for (i in 0 until bands) {
        val v = visualizer.getOrElse(i) { 0f }.coerceIn(0f, 1f)
        val pk = visualizer.getOrElse(bands + i) { 0f }.coerceIn(0f, 1f)
        val x0 = i * slotW + (slotW - barW) * 0.5f
        val h = v * (size.height - 6f)
        val col = when {
          v > 0.78f -> Signal
          v > 0.45f -> Brass
          else -> Color(0xFFACACB2)
        }
        if (h > 1f) {
          drawRect(col, Offset(x0, size.height - h), Size(barW, h))
        }
        if (pk > 0.02f) {
          val py = size.height - pk * (size.height - 6f)
          drawLine(Signal, Offset(x0, py), Offset(x0 + barW, py), strokeWidth = 1.5.dp.toPx())
        }
      }
    }
    Spacer(Modifier.height(6.dp))
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
      listOf("30HZ", "120HZ", "500HZ", "1.5KHZ", "6KHZ", "20KHZ").forEach { lbl ->
        Text(lbl, fontFamily = TechnicalMono, fontSize = 8.5.sp, color = InkMute)
      }
    }
  }
}

@Composable
fun ChannelMeterBar(
  label: String,
  peak: Float,
  rms: Float,
  hold: Float,
  modifier: Modifier = Modifier,
) {
  val pk = peak.coerceIn(0f, 1f)
  val r = rms.coerceIn(0f, 1f)
  val h = hold.coerceIn(0f, 1f)
  val dbStr = if (pk > 0.0002f) String.format("%+5.1f", 20f * log10(pk)) else "  -∞ "

  Row(
    modifier = modifier.fillMaxWidth(),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Text(
      text = label,
      fontFamily = TechnicalMono,
      fontSize = 10.sp,
      fontWeight = FontWeight.Bold,
      color = InkDim,
      modifier = Modifier.width(34.dp),
    )
    Canvas(
      modifier = Modifier
        .weight(1f)
        .height(10.dp)
        .clip(RoundedCornerShape(2.dp))
        .background(Well)
        .border(1.dp, Rule, RoundedCornerShape(2.dp)),
    ) {
      if (pk > 0.001f) {
        val col = if (pk > 0.92f) Alert else if (pk > 0.72f) Signal else Color(0xFF4A4A54)
        drawRect(col, Offset.Zero, Size(size.width * pk, size.height))
      }
      if (r > 0.001f) {
        drawRect(Mint, Offset.Zero, Size(size.width * r, size.height))
      }
      if (h > 0.02f) {
        val hx = size.width * h
        drawLine(Ink, Offset(hx, 0f), Offset(hx, size.height), strokeWidth = 1.5.dp.toPx())
      }
    }
    Spacer(Modifier.width(8.dp))
    Text(
      text = dbStr,
      fontFamily = TechnicalMono,
      fontSize = 9.5.sp,
      color = if (pk > 0.92f) Alert else InkMute,
      modifier = Modifier.width(42.dp),
    )
  }
}

@Composable
fun MiniChannelClickableRow(
  label: String,
  peak: Float,
  rms: Float,
  hold: Float,
  muted: Boolean,
  solo: Boolean,
  onToggleMute: () -> Unit,
  onToggleSolo: () -> Unit,
) {
  Row(
    modifier = Modifier
      .fillMaxWidth()
      .padding(vertical = 3.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    ChannelMeterBar(
      label = label,
      peak = peak,
      rms = rms,
      hold = hold,
      modifier = Modifier.weight(1f),
    )
    Spacer(Modifier.width(8.dp))
    Box(
      contentAlignment = Alignment.Center,
      modifier = Modifier
        .size(28.dp)
        .clip(RoundedCornerShape(4.dp))
        .background(if (muted) Alert.copy(alpha = 0.2f) else SurfaceAlt)
        .border(1.dp, if (muted) Alert else Rule, RoundedCornerShape(4.dp))
        .clickable { onToggleMute() },
    ) {
      Text("M", fontFamily = TechnicalMono, fontSize = 10.sp, fontWeight = FontWeight.Bold, color = if (muted) Alert else InkDim)
    }
    Spacer(Modifier.width(6.dp))
    Box(
      contentAlignment = Alignment.Center,
      modifier = Modifier
        .size(28.dp)
        .clip(RoundedCornerShape(4.dp))
        .background(if (solo) Brass.copy(alpha = 0.2f) else SurfaceAlt)
        .border(1.dp, if (solo) Brass else Rule, RoundedCornerShape(4.dp))
        .clickable { onToggleSolo() },
    ) {
      Text("S", fontFamily = TechnicalMono, fontSize = 10.sp, fontWeight = FontWeight.Bold, color = if (solo) Brass else InkDim)
    }
  }
}
