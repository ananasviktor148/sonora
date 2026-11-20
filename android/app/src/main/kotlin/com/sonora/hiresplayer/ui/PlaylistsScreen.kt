package com.sonora.hiresplayer.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.sonora.hiresplayer.*
import com.sonora.hiresplayer.ui.theme.*

@Composable
fun PlaylistsScreen(
  state: SonoraState,
  onOpenAudioFiles: () -> Unit,
  onPlayTrack: (playlistId: String, index: Int) -> Unit,
  onCreatePlaylist: (name: String, copyCurrent: Boolean, seedIndex: Int) -> Unit,
  onSwitchPlaylist: (id: String) -> Unit,
  onRenamePlaylist: (id: String, newName: String) -> Unit,
  onDuplicatePlaylist: (id: String) -> Unit,
  onDeletePlaylist: (id: String) -> Unit,
  onAddTrackToPlaylist: (targetId: String, sourceIndex: Int) -> Unit,
  onMoveTrack: (from: Int, to: Int) -> Unit,
  onRemoveTrack: (index: Int) -> Unit,
  onClearActive: () -> Unit,
  onRestoreReferences: () -> Unit,
) {
  var searchQuery by remember { mutableStateOf("") }
  var showCreateDialog by remember { mutableStateOf(false) }
  var createSeedTrackIdx by remember { mutableStateOf(-1) }
  var showRenameDialog by remember { mutableStateOf(false) }
  var showPlaylistMenu by remember { mutableStateOf(false) }
  var showSwitcherMenu by remember { mutableStateOf(false) }
  var addTrackDialogIdx by remember { mutableStateOf<Int?>(null) }

  val activePl = state.activePlaylist
  val activeId = state.activePlaylistId
  val activeTracks = activePl?.tracks ?: emptyList()
  val isViewingPlaying = state.playingPlaylistId == activeId

  val filteredIndexed = remember(activeTracks, searchQuery) {
    val q = searchQuery.trim().lowercase()
    activeTracks.mapIndexedNotNull { idx, entry ->
      if (q.isEmpty() ||
        entry.title.lowercase().contains(q) ||
        entry.artist.lowercase().contains(q) ||
        entry.format.lowercase().contains(q) ||
        entry.codec.lowercase().contains(q)
      ) {
        idx to entry
      } else null
    }
  }

  Column(
    modifier = Modifier
      .fillMaxSize()
      .padding(horizontal = 16.dp, vertical = 12.dp),
  ) {
    // ── Playlist Management Card ──
    SwissSubGroup(fillColor = Surface, borderColor = Rule) {
      Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        SwissKicker("PLAYLISTS", color = Brass, modifier = Modifier.weight(1f))
        SwissPrimaryButton(
          label = "+ NEW",
          onClick = {
            createSeedTrackIdx = -1
            showCreateDialog = true
          },
        )
        Spacer(Modifier.width(6.dp))
        Box {
          SwissSecondaryButton("⋯", onClick = { showPlaylistMenu = true })
          DropdownMenu(
            expanded = showPlaylistMenu,
            onDismissRequest = { showPlaylistMenu = false },
            containerColor = SurfaceAlt,
          ) {
            DropdownMenuItem(
              text = { Text("Rename Playlist…", color = Ink, fontSize = 13.sp) },
              onClick = {
                showPlaylistMenu = false
                showRenameDialog = true
              },
            )
            DropdownMenuItem(
              text = { Text("Duplicate Playlist", color = Ink, fontSize = 13.sp) },
              onClick = {
                showPlaylistMenu = false
                onDuplicatePlaylist(activeId)
              },
            )
            DropdownMenuItem(
              text = { Text("Add Studio Reference Tracks", color = Brass, fontSize = 13.sp) },
              onClick = {
                showPlaylistMenu = false
                onRestoreReferences()
              },
            )
            DropdownMenuItem(
              text = { Text("Clear Tracks", color = InkDim, fontSize = 13.sp) },
              onClick = {
                showPlaylistMenu = false
                onClearActive()
              },
            )
            HorizontalDivider(color = Rule)
            DropdownMenuItem(
              text = { Text("Delete Playlist", color = Alert, fontSize = 13.sp) },
              onClick = {
                showPlaylistMenu = false
                onDeletePlaylist(activeId)
              },
            )
          }
        }
      }

      Spacer(Modifier.height(10.dp))

      // Full-Width Playlist Switcher
      Box(modifier = Modifier.fillMaxWidth()) {
        Row(
          modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(4.dp))
            .background(Ground)
            .border(1.dp, RuleStrong, RoundedCornerShape(4.dp))
            .clickable { showSwitcherMenu = true }
            .padding(horizontal = 12.dp, vertical = 10.dp),
          verticalAlignment = Alignment.CenterVertically,
        ) {
          Text(
            text = activePl?.name ?: "Queue",
            fontFamily = CleanSans,
            fontSize = 13.5.sp,
            fontWeight = FontWeight.SemiBold,
            color = Ink,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
          )
          Text(
            text = "${String.format("%02d", activeTracks.size)} · ${formatTime(activePl?.totalDuration ?: 0.0)}  ▾",
            fontFamily = TechnicalMono,
            fontSize = 10.5.sp,
            color = InkDim,
          )
        }

        DropdownMenu(
          expanded = showSwitcherMenu,
          onDismissRequest = { showSwitcherMenu = false },
          containerColor = SurfaceAlt,
        ) {
          state.playlists.forEach { pl ->
            val playingMarker = if (state.playingPlaylistId == pl.id && state.isPlaying) "▶ " else ""
            val isSel = pl.id == activeId
            DropdownMenuItem(
              text = {
                Text(
                  text = "$playingMarker${pl.name} (${String.format("%02d", pl.tracks.size)} · ${formatTime(pl.totalDuration)})",
                  color = if (isSel) Signal else Ink,
                  fontWeight = if (isSel) FontWeight.Bold else FontWeight.Normal,
                  fontSize = 13.sp,
                )
              },
              onClick = {
                showSwitcherMenu = false
                onSwitchPlaylist(pl.id)
              },
            )
          }
        }
      }

      Spacer(Modifier.height(10.dp))

      Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SwissPrimaryButton("+ ADD AUDIO / M3U", onClick = onOpenAudioFiles, modifier = Modifier.weight(1f))
      }
    }

    Spacer(Modifier.height(10.dp))

    // Search Filter
    OutlinedTextField(
      value = searchQuery,
      onValueChange = { searchQuery = it },
      placeholder = { Text("Filter tracks, artist, format…", color = InkMute, fontSize = 12.sp) },
      singleLine = true,
      colors = OutlinedTextFieldDefaults.colors(
        focusedBorderColor = Signal,
        unfocusedBorderColor = Rule,
        focusedContainerColor = Ground,
        unfocusedContainerColor = Ground,
        focusedTextColor = Ink,
        unfocusedTextColor = Ink,
      ),
      modifier = Modifier.fillMaxWidth(),
    )

    Spacer(Modifier.height(10.dp))

    if (activeTracks.isEmpty()) {
      SwissSubGroup(fillColor = Surface) {
        Column(
          modifier = Modifier.fillMaxWidth().padding(vertical = 18.dp),
          horizontalAlignment = Alignment.CenterHorizontally,
        ) {
          SwissKicker("PLAYLIST EMPTY", color = InkDim)
          Spacer(Modifier.height(8.dp))
          Text(
            text = "Add FLAC, DSD (DSF/DFF), WAV, ALAC, MP3, or M3U8 files from your phone storage.",
            fontSize = 12.sp,
            color = InkMute,
          )
          Spacer(Modifier.height(14.dp))
          Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SwissPrimaryButton("+ ADD FILES", onClick = onOpenAudioFiles)
            SwissSecondaryButton("STUDIO REFERENCES", onClick = onRestoreReferences, accent = Brass)
          }
        }
      }
    } else {
      LazyColumn(
        modifier = Modifier.weight(1f),
        verticalArrangement = Arrangement.spacedBy(6.dp),
      ) {
        itemsIndexed(filteredIndexed, key = { _, pair -> "${pair.first}_${pair.second.path}" }) { _, (origIdx, entry) ->
          val isCur = isViewingPlaying && state.currentIndex == origIdx
          var rowMenuOpen by remember { mutableStateOf(false) }

          Row(
            modifier = Modifier
              .fillMaxWidth()
              .clip(RoundedCornerShape(5.dp))
              .background(if (isCur) SurfaceAlt else Surface)
              .border(1.dp, if (isCur) Signal else Rule, RoundedCornerShape(5.dp))
              .clickable { onPlayTrack(activeId, origIdx) }
              .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
          ) {
            Text(
              text = if (isCur && state.isPlaying) "▶" else String.format("%02d", origIdx + 1),
              fontFamily = TechnicalMono,
              fontSize = 11.sp,
              fontWeight = FontWeight.Bold,
              color = if (isCur && state.isPlaying) Signal else if (isCur) Ink else InkMute,
              modifier = Modifier.width(24.dp),
            )
            Column(modifier = Modifier.weight(1f)) {
              Text(
                text = entry.title,
                fontSize = 13.5.sp,
                fontWeight = if (isCur) FontWeight.Bold else FontWeight.Medium,
                color = if (isCur) Ink else InkDim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
              )
              Spacer(Modifier.height(2.dp))
              Row(verticalAlignment = Alignment.CenterVertically) {
                if (entry.artist.isNotBlank()) {
                  Text(
                    text = entry.artist,
                    fontSize = 11.sp,
                    color = InkMute,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                  )
                  Spacer(Modifier.width(8.dp))
                }
                val badgeCol = when {
                  entry.isDsd -> Brass
                  entry.isHires -> Mint
                  else -> InkMute
                }
                Text(
                  text = entry.specCode(),
                  fontFamily = TechnicalMono,
                  fontSize = 9.5.sp,
                  color = badgeCol,
                )
              }
            }
            Spacer(Modifier.width(8.dp))
            if (entry.duration > 0.0) {
              Text(
                text = formatTime(entry.duration),
                fontFamily = TechnicalMono,
                fontSize = 10.sp,
                color = InkMute,
              )
            }
            Spacer(Modifier.width(4.dp))
            Box {
              Text(
                text = "⋯",
                fontFamily = TechnicalMono,
                fontSize = 16.sp,
                color = InkDim,
                modifier = Modifier
                  .clip(RoundedCornerShape(4.dp))
                  .clickable { rowMenuOpen = true }
                  .padding(horizontal = 6.dp, vertical = 2.dp),
              )
              DropdownMenu(
                expanded = rowMenuOpen,
                onDismissRequest = { rowMenuOpen = false },
                containerColor = SurfaceAlt,
              ) {
                DropdownMenuItem(
                  text = { Text("Play Now", color = Ink, fontSize = 13.sp) },
                  onClick = {
                    rowMenuOpen = false
                    onPlayTrack(activeId, origIdx)
                  },
                )
                DropdownMenuItem(
                  text = { Text("Add to Playlist…", color = Brass, fontSize = 13.sp) },
                  onClick = {
                    rowMenuOpen = false
                    addTrackDialogIdx = origIdx
                  },
                )
                DropdownMenuItem(
                  text = { Text("+ New Playlist with Track", color = Signal, fontSize = 13.sp) },
                  onClick = {
                    rowMenuOpen = false
                    createSeedTrackIdx = origIdx
                    showCreateDialog = true
                  },
                )
                if (origIdx > 0) {
                  DropdownMenuItem(
                    text = { Text("Move Up", color = InkDim, fontSize = 13.sp) },
                    onClick = {
                      rowMenuOpen = false
                      onMoveTrack(origIdx, origIdx - 1)
                    },
                  )
                }
                if (origIdx + 1 < activeTracks.size) {
                  DropdownMenuItem(
                    text = { Text("Move Down", color = InkDim, fontSize = 13.sp) },
                    onClick = {
                      rowMenuOpen = false
                      onMoveTrack(origIdx, origIdx + 1)
                    },
                  )
                }
                HorizontalDivider(color = Rule)
                DropdownMenuItem(
                  text = { Text("Remove from Playlist", color = Alert, fontSize = 13.sp) },
                  onClick = {
                    rowMenuOpen = false
                    onRemoveTrack(origIdx)
                  },
                )
              }
            }
          }
        }
      }
    }
  }

  // ── Create Playlist Dialog ──
  if (showCreateDialog) {
    var nameBuf by remember { mutableStateOf("Playlist ${String.format("%02d", state.playlists.size + 1)}") }
    var copyCurrent by remember { mutableStateOf(false) }
    AlertDialog(
      onDismissRequest = { showCreateDialog = false },
      containerColor = Surface,
      title = {
        SwissKicker(
          if (createSeedTrackIdx >= 0) "NEW PLAYLIST WITH TRACK" else "CREATE NEW PLAYLIST",
          color = Signal,
        )
      },
      text = {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
          OutlinedTextField(
            value = nameBuf,
            onValueChange = { nameBuf = it },
            singleLine = true,
            label = { Text("Playlist Name", color = InkDim) },
            colors = OutlinedTextFieldDefaults.colors(
              focusedBorderColor = Signal,
              unfocusedBorderColor = RuleStrong,
              focusedTextColor = Ink,
              unfocusedTextColor = Ink,
            ),
          )
          if (createSeedTrackIdx < 0 && activeTracks.isNotEmpty()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
              Checkbox(
                checked = copyCurrent,
                onCheckedChange = { copyCurrent = it },
                colors = CheckboxDefaults.colors(checkedColor = Signal),
              )
              Text("Copy tracks from current playlist", fontSize = 12.sp, color = InkDim)
            }
          }
        }
      },
      confirmButton = {
        SwissPrimaryButton("CREATE", onClick = {
          onCreatePlaylist(nameBuf, copyCurrent, createSeedTrackIdx)
          showCreateDialog = false
        })
      },
      dismissButton = {
        SwissGhostButton("CANCEL", onClick = { showCreateDialog = false })
      },
    )
  }

  // ── Rename Playlist Dialog ──
  if (showRenameDialog) {
    var nameBuf by remember(activePl?.name) { mutableStateOf(activePl?.name ?: "") }
    AlertDialog(
      onDismissRequest = { showRenameDialog = false },
      containerColor = Surface,
      title = { SwissKicker("RENAME PLAYLIST", color = Brass) },
      text = {
        OutlinedTextField(
          value = nameBuf,
          onValueChange = { nameBuf = it },
          singleLine = true,
          label = { Text("New Name", color = InkDim) },
          colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = Signal,
            unfocusedBorderColor = RuleStrong,
            focusedTextColor = Ink,
            unfocusedTextColor = Ink,
          ),
        )
      },
      confirmButton = {
        SwissPrimaryButton("RENAME", onClick = {
          onRenamePlaylist(activeId, nameBuf)
          showRenameDialog = false
        })
      },
      dismissButton = {
        SwissGhostButton("CANCEL", onClick = { showRenameDialog = false })
      },
    )
  }

  // ── Add Track to Existing Playlist Dialog ──
  addTrackDialogIdx?.let { srcIdx ->
    AlertDialog(
      onDismissRequest = { addTrackDialogIdx = null },
      containerColor = Surface,
      title = { SwissKicker("ADD TRACK TO PLAYLIST", color = Brass) },
      text = {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
          state.playlists.forEach { pl ->
            Row(
              modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(4.dp))
                .background(SurfaceAlt)
                .border(1.dp, Rule, RoundedCornerShape(4.dp))
                .clickable {
                  onAddTrackToPlaylist(pl.id, srcIdx)
                  addTrackDialogIdx = null
                }
                .padding(horizontal = 12.dp, vertical = 10.dp),
              verticalAlignment = Alignment.CenterVertically,
            ) {
              Text(pl.name, color = Ink, fontSize = 13.sp, modifier = Modifier.weight(1f))
              Text("${pl.tracks.size}", fontFamily = TechnicalMono, fontSize = 11.sp, color = InkMute)
            }
          }
        }
      },
      confirmButton = {},
      dismissButton = { SwissGhostButton("CLOSE", onClick = { addTrackDialogIdx = null }) },
    )
  }
}
