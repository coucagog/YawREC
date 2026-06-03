package app.yawrec.mobile.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.yawrec.mobile.ConnectionState
import app.yawrec.mobile.DesktopStatus
import app.yawrec.mobile.RemoteViewModel
import app.yawrec.mobile.ui.theme.*

@Composable
fun RemoteScreen(viewModel: RemoteViewModel) {
    val ip          by viewModel.ip.collectAsStateWithLifecycle()
    val connState   by viewModel.connState.collectAsStateWithLifecycle()
    val status      by viewModel.desktopStatus.collectAsStateWithLifecycle()
    val stoppedPath by viewModel.stoppedPath.collectAsStateWithLifecycle()

    val isConnected  = connState is ConnectionState.Connected
    val isConnecting = connState is ConnectionState.Connecting

    Column(
        Modifier
            .fillMaxSize()
            .background(Brush.verticalGradient(listOf(Color(0xFF1B5E20), Color(0xFF004D40))))
            .padding(horizontal = 20.dp, vertical = 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            "Contrôle à distance",
            fontSize = 18.sp, fontWeight = FontWeight.Medium, color = Color.White,
        )

        // ── Panneau de connexion ──────────────────────────────────────────────
        Column(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(20.dp))
                .background(SurfaceDark)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Adresse IP du bureau", fontSize = 13.sp, color = Muted)

            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedTextField(
                    value = ip,
                    onValueChange = viewModel::onIpChanged,
                    modifier = Modifier.weight(1f),
                    placeholder = { Text("192.168.1.x", color = Muted, fontSize = 14.sp) },
                    singleLine = true,
                    enabled = !isConnected && !isConnecting,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedTextColor      = OnSurface,
                        unfocusedTextColor    = OnSurface,
                        focusedContainerColor = SurfaceVariant,
                        unfocusedContainerColor = SurfaceVariant,
                        disabledContainerColor = SurfaceVariant,
                        disabledTextColor     = OnSurface,
                        focusedBorderColor    = PrimaryPurple,
                        unfocusedBorderColor  = Divider,
                        disabledBorderColor   = Divider,
                    ),
                    shape = RoundedCornerShape(12.dp),
                )

                Button(
                    onClick = if (isConnected) viewModel::disconnect else viewModel::connect,
                    shape = RoundedCornerShape(12.dp),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = if (isConnected) Rec500 else PrimaryPurple,
                    ),
                    modifier = Modifier.height(52.dp),
                ) {
                    if (isConnecting) {
                        CircularProgressIndicator(
                            Modifier.size(16.dp), color = Color.White, strokeWidth = 2.dp,
                        )
                    } else {
                        Text(if (isConnected) "Couper" else "Connecter", fontWeight = FontWeight.Medium)
                    }
                }
            }

            // Badge d'état
            when (val s = connState) {
                is ConnectionState.Connected    -> ConnBadge("Connecté · port 9799", Ok500)
                is ConnectionState.Connecting   -> ConnBadge("Connexion en cours…", Color(0xFFFFA726))
                is ConnectionState.Failed       -> ConnBadge(s.message, Rec500)
                is ConnectionState.Disconnected -> {}
            }
        }

        // ── Contrôles bureau (affiché quand connecté) ────────────────────────
        if (isConnected) {
            DesktopControlCard(
                status = status,
                onStart      = { viewModel.send("start") },
                onStop       = { viewModel.send("stop") },
                onPauseResume = {
                    if (status.phase == "recording") viewModel.send("pause")
                    else viewModel.send("resume")
                },
            )
        }

        // Dernier fichier enregistré
        val path = stoppedPath
        if (!path.isNullOrEmpty()) {
            Text(
                "Fichier : …${path.takeLast(45)}",
                fontSize = 11.sp, color = Muted,
            )
        }
    }
}

// ── Carte de contrôle du bureau ───────────────────────────────────────────────

@Composable
private fun DesktopControlCard(
    status: DesktopStatus,
    onStart: () -> Unit,
    onStop: () -> Unit,
    onPauseResume: () -> Unit,
) {
    val isIdle      = status.phase == "idle"
    val isRecording = status.phase == "recording"
    val isPaused    = status.phase == "paused"
    val isActive    = !isIdle

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(20.dp))
            .background(SurfaceDark)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        // En-tête
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(10.dp)
                    .clip(CircleShape)
                    .background(
                        when {
                            isRecording -> Rec500
                            isPaused    -> Color(0xFFFFA726)
                            else        -> Color(0xFF555566)
                        }
                    )
            )
            Spacer(Modifier.width(8.dp))
            Text(
                when {
                    isRecording -> "Bureau · Enregistrement"
                    isPaused    -> "Bureau · En pause"
                    else        -> "Bureau · Inactif"
                },
                fontSize = 13.sp, fontWeight = FontWeight.Medium, color = OnSurface,
            )
            Spacer(Modifier.weight(1f))
            if (isActive) {
                Text(status.sizeHuman, fontSize = 11.sp, color = Muted)
            }
        }

        // Chronomètre
        if (isActive) {
            Text(
                status.elapsed,
                fontSize = 34.sp,
                fontWeight = FontWeight.Light,
                color = Color.White,
                fontFamily = FontFamily.Monospace,
            )
        }

        // Boutons
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = if (isActive) onStop else onStart,
                modifier = Modifier.weight(1f).height(46.dp),
                shape = RoundedCornerShape(14.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = if (isActive) Rec500 else PrimaryPurple,
                ),
            ) {
                if (isActive) {
                    Box(
                        Modifier
                            .size(10.dp)
                            .clip(RoundedCornerShape(2.dp))
                            .background(Color.White)
                    )
                    Spacer(Modifier.width(8.dp))
                    Text("Arrêter", fontWeight = FontWeight.Medium)
                } else {
                    Text("Démarrer", fontWeight = FontWeight.Medium)
                }
            }

            if (isActive) {
                Box(
                    Modifier
                        .size(46.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(SurfaceVariant)
                        .clickable(onClick = onPauseResume),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = if (isPaused) Icons.Filled.PlayArrow else Icons.Filled.Pause,
                        contentDescription = if (isPaused) "Reprendre" else "Pause",
                        tint = OnSurface,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
        }
    }
}

// ── Indicateur de connexion ───────────────────────────────────────────────────

@Composable
private fun ConnBadge(text: String, color: Color) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(Modifier.size(7.dp).clip(CircleShape).background(color))
        Text(text, fontSize = 12.sp, color = color)
    }
}
