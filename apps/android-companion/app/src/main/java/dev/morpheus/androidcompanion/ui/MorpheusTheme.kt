package dev.morpheus.androidcompanion.ui

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val AppColors: ColorScheme = darkColorScheme(
    primary = Color(0xFF5ED37A),
    onPrimary = Color(0xFF072914),
    primaryContainer = Color(0xFF174A2B),
    onPrimaryContainer = Color(0xFFD8F8DF),
    secondary = Color(0xFFC9D1DA),
    onSecondary = Color(0xFF1C252D),
    secondaryContainer = Color(0xFF29343D),
    onSecondaryContainer = Color(0xFFE5ECF2),
    tertiary = Color(0xFFFFCA67),
    onTertiary = Color(0xFF3B2600),
    background = Color(0xFF111820),
    onBackground = Color(0xFFE8EDF2),
    surface = Color(0xFF111820),
    onSurface = Color(0xFFE8EDF2),
    surfaceVariant = Color(0xFF1A232C),
    onSurfaceVariant = Color(0xFFAEB8C2),
    outline = Color(0xFF3A4650),
    error = Color(0xFFFF6B6B),
    onError = Color(0xFF3F0909),
    errorContainer = Color(0xFF4A1717),
    onErrorContainer = Color(0xFFFFDADA),
)

@Composable
fun MorpheusTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = AppColors,
        content = content,
    )
}
