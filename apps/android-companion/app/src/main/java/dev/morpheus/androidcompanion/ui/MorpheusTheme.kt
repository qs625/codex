package dev.morpheus.androidcompanion.ui

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val LightColors: ColorScheme = lightColorScheme(
    primary = Color(0xFF255C55),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFD5ECE7),
    onPrimaryContainer = Color(0xFF0B2F2B),
    secondary = Color(0xFF6E5F2E),
    secondaryContainer = Color(0xFFEFE2AD),
    tertiary = Color(0xFF76536B),
    background = Color(0xFFFAFAF7),
    surface = Color(0xFFFAFAF7),
    surfaceVariant = Color(0xFFE6E9E5),
    error = Color(0xFFB3261E),
)

@Composable
fun MorpheusTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = LightColors,
        content = content,
    )
}
