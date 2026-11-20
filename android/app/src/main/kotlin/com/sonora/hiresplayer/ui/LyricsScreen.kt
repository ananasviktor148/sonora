package com.sonora.hiresplayer.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sonora.hiresplayer.*
import com.sonora.hiresplayer.ui.theme.*

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun LyricsScreen(
  state: SonoraState,
  fetchingOnline: Boolean,
  onUploadLyricsFile: () -> Unit,
  onFetchOnlineLyrics: () -> Unit,
  onSaveLyricsText: (String) -> Unit,
  onSetOffsetMs: (Long) -> Unit,
  onSeek: (Double) -> Unit,
  onUpdateFontSize: (Float) -> Unit,
  onToggleAutoScroll: () -> Unit,
) {
  var editing by remember { mutableStateOf(false) }
  var editBuf by remember { mutableStateOf("") }
  val lyrics = state.lyrics
  val listState = rememberLazyListState()

  val activeLine = remember(lyrics, state.position, state.lyricsOffsetMs) {
    lyrics?.activeLineIndex(state.position, state.lyricsOffsetMs) ?: -1
  }

  LaunchedEffect(activeLine, state.config.lyricsAutoScroll) {
    if (state.config.lyricsAutoScroll && activeLine >= 0 && !editing) {
      val target = (activeLine - 2).coerceAtLeast(0)
      listState.animateScrollToItem(target)
    }
  }

  Column(
    modifier = Modifier
      .fillMaxSize()
      .padding(horizontal = 16.dp, vertical = 12.dp),
  ) {
    // ── Lyrics Studio Controls Header ──
    SwissSubGroup(fillColor = Surface, borderColor = Rule) {
      Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        SwissKicker("LYRICS / TRANSCRIPT", color = Brass, modifier = Modifier.weight(1f))
        if (lyrics != null) {
          Text(
            text = lyrics.source.uppercase(),
            fontFamily = TechnicalMono,
            fontSize = 9.5.sp,
            color = InkMute,
          )
        }
      }
      Spacer(Modifier.height(10.dp))
      FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
      ) {
        SwissPrimaryButton("↑ UPLOAD .LRC/.SRT/.TXT", onClick = onUploadLyricsFile)
        SwissSecondaryButton(
          label = if (fetchingOnline) "SEARCHING…" else "FETCH ONLINE",
          onClick = { if (!fetchingOnline) onFetchOnlineLyrics() },
          accent = Brass,
        )
        SwissSecondaryButton(
          label = if (editing) "CLOSE EDITOR" else "EDIT / STAMP",
          onClick = {
            if (!editing) {
              editBuf = lyrics?.rawText ?: ""
            }
            editing = !editing
          },
        )
      }

      Spacer(Modifier.height(10.dp))
      SwissHairline()
      Spacer(Modifier.height(8.dp))

      // Sync Offset + Typography Size Controls
      Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
      ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          SwissGhostButton("-0.5s", onClick = { onSetOffsetMs(state.lyricsOffsetMs - 500) })
          Text(
            text = String.format("%+.1fs", state.lyricsOffsetMs / 1000.0),
            fontFamily = TechnicalMono,
            fontSize = 10.sp,
            color = if (state.lyricsOffsetMs != 0L) Signal else InkDim,
            modifier = Modifier.padding(horizontal = 4.dp),
          )
          SwissGhostButton("+0.5s", onClick = { onSetOffsetMs(state.lyricsOffsetMs + 500) })
        }

        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
          SwissGhostButton("A-", onClick = { onUpdateFontSize((state.config.lyricsFontSize - 2f).coerceAtLeast(12f)) })
          SwissGhostButton("A+", onClick = { onUpdateFontSize((state.config.lyricsFontSize + 2f).coerceAtMost(28f)) })
          SwissToggleChip(
            label = "SYNC",
            selected = state.config.lyricsAutoScroll,
            accent = Mint,
            onClick = onToggleAutoScroll,
          )
        }
      }
    }

    Spacer(Modifier.height(12.dp))

    if (editing) {
      // ── Inline LRC / Lyrics Editor with Live Timestamp Stamper ──
      Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
      ) {
        SwissSecondaryButton(
          label = "+ STAMP [${formatLrcStamp(state.position)}]",
          onClick = {
            val stamp = "[${formatLrcStamp(state.position)}] "
            editBuf = if (editBuf.isEmpty() || editBuf.endsWith("\n")) {
              editBuf + stamp
            } else {
              "$editBuf\n$stamp"
            }
          },
          accent = Brass,
          modifier = Modifier.weight(1f),
        )
        SwissPrimaryButton(
          label = "SAVE LYRICS",
          onClick = {
            onSaveLyricsText(editBuf)
            editing = false
          },
        )
      }
      Spacer(Modifier.height(8.dp))
      OutlinedTextField(
        value = editBuf,
        onValueChange = { editBuf = it },
        placeholder = { Text("Paste lyrics or LRC lines like [00:12.50] First line…", color = InkMute) },
        colors = OutlinedTextFieldDefaults.colors(
          focusedBorderColor = Signal,
          unfocusedBorderColor = Rule,
          focusedContainerColor = Ground,
          unfocusedContainerColor = Ground,
          focusedTextColor = Ink,
          unfocusedTextColor = Ink,
        ),
        modifier = Modifier
          .fillMaxWidth()
          .weight(1f),
      )
    } else if (lyrics == null || (!lyrics.isSynced && lyrics.plainLines.isEmpty())) {
      SwissSubGroup(fillColor = Surface) {
        Column(
          modifier = Modifier.fillMaxWidth().padding(vertical = 24.dp),
          horizontalAlignment = Alignment.CenterHorizontally,
        ) {
          SwissKicker("NO LYRICS LOADED", color = InkDim)
          Spacer(Modifier.height(8.dp))
          Text(
            text = "Upload an .lrc, .srt, .vtt, or .txt lyrics file, fetch synchronized lyrics from LRCLIB, or paste and timestamp your own.",
            fontSize = 12.5.sp,
            color = InkMute,
          )
          Spacer(Modifier.height(16.dp))
          Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SwissPrimaryButton("↑ UPLOAD FILE", onClick = onUploadLyricsFile)
            SwissSecondaryButton("FETCH ONLINE", onClick = onFetchOnlineLyrics, accent = Brass)
          }
        }
      }
    } else if (lyrics.isSynced) {
      val fontSp = state.config.lyricsFontSize.coerceIn(12f, 28f).sp
      LazyColumn(
        state = listState,
        modifier = Modifier
          .fillMaxWidth()
          .weight(1f)
          .clip(RoundedCornerShape(6.dp))
          .background(Surface)
          .border(1.dp, Rule, RoundedCornerShape(6.dp))
          .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
      ) {
        itemsIndexed(lyrics.syncedLines) { idx, line ->
          val isCurrent = idx == activeLine
          Row(
            modifier = Modifier
              .fillMaxWidth()
              .clip(RoundedCornerShape(4.dp))
              .background(if (isCurrent) SurfaceAlt else androidx.compose.ui.graphics.Color.Transparent)
              .border(
                width = 1.dp,
                color = if (isCurrent) Signal else androidx.compose.ui.graphics.Color.Transparent,
                shape = RoundedCornerShape(4.dp),
              )
              .clickable { onSeek(line.timeSecs) }
              .padding(horizontal = 10.dp, vertical = 8.dp),
            verticalAlignment = Alignment.Top,
          ) {
            Text(
              text = formatTime(line.timeSecs),
              fontFamily = TechnicalMono,
              fontSize = 9.5.sp,
              color = if (isCurrent) Signal else InkMute,
              modifier = Modifier.padding(top = 4.dp, end = 10.dp),
            )
            Text(
              text = line.text.ifBlank { "· · ·" },
              fontFamily = if (isCurrent) EditorialSerif else CleanSans,
              fontSize = if (isCurrent) (state.config.lyricsFontSize + 2f).coerceAtMost(30f).sp else fontSp,
              fontWeight = if (isCurrent) FontWeight.SemiBold else FontWeight.Normal,
              color = if (isCurrent) Ink else InkDim,
              lineHeight = (state.config.lyricsFontSize + 8f).sp,
            )
          }
        }
      }
    } else {
      val fontSp = state.config.lyricsFontSize.coerceIn(12f, 28f).sp
      LazyColumn(
        modifier = Modifier
          .fillMaxWidth()
          .weight(1f)
          .clip(RoundedCornerShape(6.dp))
          .background(Surface)
          .border(1.dp, Rule, RoundedCornerShape(6.dp))
          .padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
      ) {
        itemsIndexed(lyrics.plainLines) { _, line ->
          Text(
            text = line,
            fontFamily = CleanSans,
            fontSize = fontSp,
            color = InkDim,
            lineHeight = (state.config.lyricsFontSize + 7f).sp,
          )
        }
      }
    }
  }
}

private fun formatLrcStamp(secs: Double): String {
  val totalMs = (secs.coerceAtLeast(0.0) * 1000.0).toLong()
  val m = totalMs / 60_000
  val s = (totalMs % 60_000) / 1000
  val cs = (totalMs % 1000) / 10
  return String.format("%02d:%02d.%02d", m, s, cs)
}
