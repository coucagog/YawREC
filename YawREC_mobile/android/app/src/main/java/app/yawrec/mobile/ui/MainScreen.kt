package app.yawrec.mobile.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.*
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.Videocam
import androidx.compose.material.icons.outlined.DesktopWindows
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.yawrec.mobile.RecordingUiState
import app.yawrec.mobile.RecordingViewModel
import app.yawrec.mobile.ui.theme.*
import kotlin.math.pow

@Composable
fun MainScreen(
    viewModel: RecordingViewModel,
    onStartRecording: () -> Unit,
    onStopRecording: () -> Unit,
    onPauseResume: () -> Unit,
    onToggleMic: () -> Unit,
    onToggleCamera: () -> Unit,
) {
    val state         by viewModel.state.collectAsStateWithLifecycle()
    val vuLevel       by viewModel.vuLevel.collectAsStateWithLifecycle()
    val micEnabled    by viewModel.micEnabled.collectAsStateWithLifecycle()
    val cameraEnabled by viewModel.cameraEnabled.collectAsStateWithLifecycle()

    Box(
        Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(listOf(Color(0xFF1B5E20), Color(0xFF004D40)))
            )
    ) {
        // Ongoing notification banner (when recording / paused)
        AnimatedVisibility(
            visible = state !is RecordingUiState.Idle,
            modifier = Modifier.align(Alignment.TopCenter).padding(top = 48.dp),
            enter = expandVertically(),
            exit = shrinkVertically()
        ) {
            OngoingBanner(
                state       = state,
                vuLevel     = vuLevel,
                onStop      = onStopRecording,
                onPauseResume = onPauseResume,
            )
        }

        // Bottom sheet always visible
        Box(Modifier.align(Alignment.BottomCenter)) {
            RecordingSheet(
                state         = state,
                micEnabled    = micEnabled,
                cameraEnabled = cameraEnabled,
                onStart       = onStartRecording,
                onStop        = onStopRecording,
                onPauseResume = onPauseResume,
                onToggleMic   = onToggleMic,
                onToggleCamera = onToggleCamera,
            )
        }
    }
}

// ── Ongoing notification banner (mirrors the and-notif design) ──────────────

@Composable
fun OngoingBanner(
    state: RecordingUiState,
    vuLevel: Float,
    onStop: () -> Unit,
    onPauseResume: () -> Unit,
) {
    val elapsed = when (state) {
        is RecordingUiState.Recording -> formatElapsed(state.elapsedMs)
        is RecordingUiState.Paused    -> formatElapsed(state.elapsedMs)
        else -> "00:00:00"
    }
    val fileSize = when (state) {
        is RecordingUiState.Recording -> state.fileSizeBytes
        is RecordingUiState.Paused    -> state.fileSizeBytes
        else -> 0L
    }
    val isPaused = state is RecordingUiState.Paused

    Column(
        Modifier
            .padding(horizontal = 16.dp)
            .clip(RoundedCornerShape(28.dp))
            .background(Color(0xFF2B2B2E))
            .padding(16.dp)
    ) {
        // Header
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(24.dp)
                    .clip(CircleShape)
                    .background(Rec500),
                contentAlignment = Alignment.Center
            ) {
                Box(
                    Modifier
                        .size(8.dp)
                        .clip(RoundedCornerShape(2.dp))
                        .background(Color.White)
                )
            }
            Spacer(Modifier.width(10.dp))
            Column {
                Text("YawREC", fontSize = 13.sp, fontWeight = FontWeight.Medium, color = OnSurface)
                Text(
                    if (isPaused) "En pause · ${formatSize(fileSize)}" else "Enregistrement · ${formatSize(fileSize)}",
                    fontSize = 11.sp, color = Muted, modifier = Modifier.padding(top = 2.dp)
                )
            }
        }
        Spacer(Modifier.height(10.dp))

        // Timer row
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                elapsed,
                fontSize = 28.sp,
                fontWeight = FontWeight.Light,
                color = Color.White,
                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
            )
            Spacer(Modifier.weight(1f))
            VuMeter(level = vuLevel)
            Spacer(Modifier.width(10.dp))
            Text("MP4 · 1080p", fontSize = 12.sp, color = Muted)
        }
        Spacer(Modifier.height(14.dp))

        // Action chips
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            // Stop chip (tonal, fills remaining width)
            Button(
                onClick = onStop,
                modifier = Modifier.weight(1f).height(32.dp),
                shape = RoundedCornerShape(999.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Rec500),
                contentPadding = PaddingValues(horizontal = 12.dp)
            ) {
                Icon(Icons.Filled.Stop, null, Modifier.size(14.dp))
                Spacer(Modifier.width(6.dp))
                Text("Arrêter", fontSize = 12.sp, fontWeight = FontWeight.Medium)
            }
            // Pause icon chip
            ChipIconButton(
                icon = if (isPaused) Icons.Filled.PlayArrow else Icons.Filled.Pause,
                contentDescription = if (isPaused) "Reprendre" else "Pause",
                onClick = onPauseResume
            )
            // Mic icon chip
            ChipIconButton(icon = Icons.Filled.Mic, contentDescription = "Micro", onClick = {})
        }
    }
}

@Composable
fun ChipIconButton(icon: ImageVector, contentDescription: String, onClick: () -> Unit) {
    Box(
        Modifier
            .size(32.dp)
            .clip(CircleShape)
            .background(Color(0xFF4A4458))
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        Icon(icon, contentDescription, Modifier.size(14.dp), tint = Color(0xFFE8DEF8))
    }
}

// ── Bottom sheet (mirrors the and-sheet design) ──────────────────────────────

@Composable
fun RecordingSheet(
    state: RecordingUiState,
    micEnabled: Boolean,
    cameraEnabled: Boolean,
    onStart: () -> Unit,
    onStop: () -> Unit,
    onPauseResume: () -> Unit,
    onToggleMic: () -> Unit,
    onToggleCamera: () -> Unit,
) {
    val isActive = state !is RecordingUiState.Idle
    val isPaused = state is RecordingUiState.Paused

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
            .background(SurfaceDark)
            .padding(horizontal = 20.dp)
            .padding(top = 12.dp, bottom = 36.dp)
    ) {
        // Grabber
        Box(
            Modifier
                .align(Alignment.CenterHorizontally)
                .size(32.dp, 4.dp)
                .clip(RoundedCornerShape(2.dp))
                .background(Divider)
        )
        Spacer(Modifier.height(14.dp))

        Text("YawREC", fontSize = 16.sp, fontWeight = FontWeight.Medium, color = OnSurface)
        Spacer(Modifier.height(4.dp))
        Text(
            when (state) {
                is RecordingUiState.Idle      -> "Prêt à enregistrer"
                is RecordingUiState.Recording -> "Enregistrement · ${formatElapsed(state.elapsedMs)}  ·  ${formatSize(state.fileSizeBytes)}"
                is RecordingUiState.Paused    -> "En pause · ${formatElapsed(state.elapsedMs)}  ·  ${formatSize(state.fileSizeBytes)}"
            },
            fontSize = 12.sp, color = Muted
        )
        Spacer(Modifier.height(14.dp))

        // Tile row (visible when recording or paused)
        AnimatedVisibility(visible = isActive) {
            Column {
                TileRow(
                    micEnabled    = micEnabled,
                    cameraEnabled = cameraEnabled,
                    onToggleMic   = onToggleMic,
                    onToggleCamera = onToggleCamera,
                )
                Spacer(Modifier.height(14.dp))
            }
        }

        // FAB row
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(
                onClick = if (isActive) onStop else onStart,
                modifier = Modifier.weight(1f).height(48.dp),
                shape = RoundedCornerShape(16.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Rec500)
            ) {
                if (isActive) {
                    Box(
                        Modifier
                            .size(12.dp)
                            .clip(RoundedCornerShape(3.dp))
                            .background(Color.White)
                    )
                    Spacer(Modifier.width(8.dp))
                    Text("Arrêter l'enregistrement", fontSize = 14.sp, fontWeight = FontWeight.Medium)
                } else {
                    Text("Démarrer l'enregistrement", fontSize = 14.sp, fontWeight = FontWeight.Medium)
                }
            }

            if (isActive) {
                Box(
                    Modifier
                        .size(48.dp)
                        .clip(RoundedCornerShape(14.dp))
                        .background(SurfaceVariant)
                        .clickable(onClick = onPauseResume),
                    contentAlignment = Alignment.Center
                ) {
                    Icon(
                        if (isPaused) Icons.Filled.PlayArrow else Icons.Filled.Pause,
                        if (isPaused) "Reprendre" else "Pause",
                        tint = OnSurface,
                        modifier = Modifier.size(20.dp)
                    )
                }

                Box(
                    Modifier
                        .size(48.dp)
                        .clip(RoundedCornerShape(14.dp))
                        .background(SurfaceVariant)
                        .clickable { /* more options */ },
                    contentAlignment = Alignment.Center
                ) {
                    Icon(Icons.Filled.MoreVert, "Plus d'options", tint = OnSurface, modifier = Modifier.size(20.dp))
                }
            }
        }
    }
}

// ── VU meter ─────────────────────────────────────────────────────────────────

/**
 * Three bars whose heights track `level` (0..1) with different per-bar multipliers
 * so they look staggered, matching the CSS design.  When level == 0 the bars sit at
 * their minimum height (simulating silence) instead of collapsing completely.
 */
@Composable
fun VuMeter(level: Float, modifier: Modifier = Modifier) {
    // Apply a sqrt curve so quiet signals are still visible
    val curved = level.pow(0.5f)

    // Each bar animates independently with its own spring stiffness
    data class BarSpec(val mult: Float, val stiffness: Float, val maxDp: Dp)
    val bars = listOf(
        BarSpec(0.65f, Spring.StiffnessMedium,     12.dp),
        BarSpec(1.00f, Spring.StiffnessMediumLow,  16.dp),
        BarSpec(0.80f, Spring.StiffnessLow,        14.dp),
    )

    Row(
        modifier = modifier.height(16.dp),
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        bars.forEach { spec ->
            val target = maxOf(0.18f, curved * spec.mult)  // floor at 18% for visible silence
            val animated by animateFloatAsState(
                targetValue = target,
                animationSpec = spring(stiffness = spec.stiffness),
                label = "vu",
            )
            Box(
                Modifier
                    .width(3.dp)
                    .fillMaxHeight(animated)
                    .clip(RoundedCornerShape(1.5.dp))
                    .background(Ok500)
            )
        }
    }
}

// ── Tile row (Écran / Micro / Caméra) ────────────────────────────────────────

@Composable
fun TileRow(
    micEnabled: Boolean,
    cameraEnabled: Boolean,
    onToggleMic: () -> Unit,
    onToggleCamera: () -> Unit,
) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {

        // Écran — always active, not tappable
        Tile(
            label   = "Écran",
            icon    = Icons.Outlined.DesktopWindows,
            active  = true,
            enabled = false,   // screen recording can't be toggled
            onClick = {},
        )

        // Micro — toggleable, fully functional
        Tile(
            label   = "Micro",
            icon    = if (micEnabled) Icons.Filled.Mic else Icons.Filled.Mic,
            active  = micEnabled,
            enabled = true,
            onClick = onToggleMic,
        )

        // Caméra — toggleable (PiP compositing: future sprint)
        Tile(
            label   = "Caméra",
            icon    = Icons.Outlined.Videocam,
            active  = cameraEnabled,
            enabled = true,
            onClick = onToggleCamera,
        )
    }
}

@Composable
private fun RowScope.Tile(
    label: String,
    icon: ImageVector,
    active: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    val bgColor    = if (active) SurfaceActive else SurfaceVariant
    val tintColor  = if (active) PrimaryPurple else OnSurfaceVar
    val alpha      = if (enabled) 1f else 0.5f

    Column(
        Modifier
            .weight(1f)
            .aspectRatio(1.2f)
            .clip(RoundedCornerShape(16.dp))
            .background(bgColor)
            .then(if (enabled) Modifier.clickable(onClick = onClick) else Modifier),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(
            icon, label,
            Modifier.size(20.dp),
            tint = tintColor.copy(alpha = alpha),
        )
        Spacer(Modifier.height(4.dp))
        Text(
            label,
            fontSize  = 10.sp,
            color     = tintColor.copy(alpha = alpha),
        )
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

private fun formatElapsed(ms: Long): String {
    val s = ms / 1000
    return "%02d:%02d:%02d".format(s / 3600, (s % 3600) / 60, s % 60)
}

private fun formatSize(bytes: Long): String = when {
    bytes < 1024 * 1024 -> "%.1f KB".format(bytes / 1024.0)
    else -> "%.2f MB".format(bytes / (1024.0 * 1024))
}
