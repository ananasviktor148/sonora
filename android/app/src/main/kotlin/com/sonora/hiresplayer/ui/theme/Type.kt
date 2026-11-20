package com.sonora.hiresplayer.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

val EditorialSerif = FontFamily.Serif
val TechnicalMono = FontFamily.Monospace
val CleanSans = FontFamily.SansSerif

val SonoraTypography = Typography(
  displaySmall = TextStyle(
    fontFamily = EditorialSerif,
    fontWeight = FontWeight.Normal,
    fontSize = 26.sp,
    lineHeight = 32.sp,
    color = Ink,
  ),
  headlineSmall = TextStyle(
    fontFamily = EditorialSerif,
    fontWeight = FontWeight.Normal,
    fontSize = 18.sp,
    lineHeight = 24.sp,
    color = InkDim,
  ),
  titleMedium = TextStyle(
    fontFamily = CleanSans,
    fontWeight = FontWeight.SemiBold,
    fontSize = 14.sp,
    lineHeight = 20.sp,
    color = Ink,
  ),
  bodyMedium = TextStyle(
    fontFamily = CleanSans,
    fontWeight = FontWeight.Normal,
    fontSize = 13.sp,
    lineHeight = 18.sp,
    color = InkDim,
  ),
  labelSmall = TextStyle(
    fontFamily = TechnicalMono,
    fontWeight = FontWeight.SemiBold,
    fontSize = 10.sp,
    letterSpacing = 0.8.sp,
    color = InkMute,
  ),
)
