package app.yawrec.mobile.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val YawRECColorScheme = darkColorScheme(
    primary          = PrimaryPurple,
    primaryContainer = SurfaceActive,
    secondary        = Ok500,
    background       = Background,
    surface          = SurfaceDark,
    surfaceVariant   = SurfaceVariant,
    onBackground     = OnSurface,
    onSurface        = OnSurface,
    onSurfaceVariant = OnSurfaceVar,
    error            = Rec500,
    onPrimary        = Color.White,
)

@Composable
fun YawRECTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = YawRECColorScheme,
        content = content
    )
}
