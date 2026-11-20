package com.sonora.hiresplayer.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable

private val SonoraDarkColorScheme = darkColorScheme(
  primary = Signal,
  onPrimary = Ground,
  secondary = Brass,
  onSecondary = Ground,
  tertiary = Mint,
  background = Ground,
  onBackground = Ink,
  surface = Surface,
  onSurface = Ink,
  surfaceVariant = SurfaceAlt,
  onSurfaceVariant = InkDim,
  outline = Rule,
  outlineVariant = RuleStrong,
  error = Alert,
  onError = Ink,
)

@Composable
fun SonoraTheme(content: @Composable () -> Unit) {
  MaterialTheme(
    colorScheme = SonoraDarkColorScheme,
    typography = SonoraTypography,
    content = content,
  )
}
