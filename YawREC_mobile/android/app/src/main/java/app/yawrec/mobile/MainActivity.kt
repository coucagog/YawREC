package app.yawrec.mobile

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.outlined.DesktopWindows
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.yawrec.mobile.recording.RecordingService
import app.yawrec.mobile.ui.MainScreen
import app.yawrec.mobile.ui.RemoteScreen
import app.yawrec.mobile.ui.theme.YawRECTheme

class MainActivity : ComponentActivity() {

    private val viewModel: RecordingViewModel by viewModels()
    private val remoteViewModel: RemoteViewModel by viewModels()

    private val projectionManager by lazy {
        getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
    }

    private val projectionLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK && result.data != null) {
            launchRecordingService(result.resultCode, result.data!!)
        }
    }

    private val permissionsLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { granted ->
        if (granted.values.all { it }) requestProjectionPermission()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            YawRECTheme {
                var selectedTab by remember { mutableIntStateOf(0) }

                Column(
                    Modifier
                        .fillMaxSize()
                        .background(Color(0xFF1B5E20))
                ) {
                    // ── Barre d'onglets ──────────────────────────────────────
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .statusBarsPadding()
                            .padding(horizontal = 16.dp, vertical = 8.dp),
                        horizontalArrangement = Arrangement.Center,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        TabChip(
                            label = "Enregistrer",
                            icon = Icons.Filled.Videocam,
                            selected = selectedTab == 0,
                            onClick = { selectedTab = 0 },
                        )
                        Spacer(Modifier.width(8.dp))
                        TabChip(
                            label = "Bureau",
                            icon = Icons.Outlined.DesktopWindows,
                            selected = selectedTab == 1,
                            onClick = { selectedTab = 1 },
                        )
                    }

                    // ── Contenu ──────────────────────────────────────────────
                    Box(Modifier.weight(1f)) {
                        when (selectedTab) {
                            0 -> MainScreen(
                                viewModel        = viewModel,
                                onStartRecording = ::checkPermissionsThenRecord,
                                onStopRecording  = ::stopRecording,
                                onPauseResume    = ::pauseResumeRecording,
                                onToggleMic      = ::toggleMic,
                                onToggleCamera   = ::toggleCamera,
                            )
                            1 -> RemoteScreen(remoteViewModel)
                        }
                    }
                }
            }
        }
    }

    // ── Enregistrement local ─────────────────────────────────────────────────

    private fun checkPermissionsThenRecord() {
        val missing = buildList {
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED)
                add(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
                add(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (missing.isEmpty()) requestProjectionPermission()
        else permissionsLauncher.launch(missing.toTypedArray())
    }

    private fun requestProjectionPermission() {
        projectionLauncher.launch(projectionManager.createScreenCaptureIntent())
    }

    private fun launchRecordingService(resultCode: Int, data: Intent) {
        Intent(this, RecordingService::class.java).apply {
            action = RecordingService.ACTION_START
            putExtra(RecordingService.EXTRA_RESULT_CODE, resultCode)
            putExtra(RecordingService.EXTRA_PROJECTION_DATA, data)
        }.also { startForegroundService(it) }
    }

    private fun stopRecording() {
        Intent(this, RecordingService::class.java).apply {
            action = RecordingService.ACTION_STOP
        }.also { startService(it) }
    }

    private fun pauseResumeRecording() {
        val action = when (RecordingState.phase.value) {
            RecordingPhase.Recording -> RecordingService.ACTION_PAUSE
            RecordingPhase.Paused    -> RecordingService.ACTION_RESUME
            RecordingPhase.Idle      -> return
        }
        serviceIntent(action)
    }

    fun toggleMic() {
        if (RecordingState.phase.value == RecordingPhase.Idle) return
        serviceIntent(RecordingService.ACTION_TOGGLE_MIC)
    }

    fun toggleCamera() {
        if (RecordingState.phase.value == RecordingPhase.Idle) return
        if (!RecordingConfig.cameraEnabled.value &&
            checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            cameraPermLauncher.launch(Manifest.permission.CAMERA)
            return
        }
        serviceIntent(RecordingService.ACTION_TOGGLE_CAMERA)
    }

    private val cameraPermLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) serviceIntent(RecordingService.ACTION_TOGGLE_CAMERA)
    }

    private fun serviceIntent(action: String) {
        Intent(this, RecordingService::class.java).apply {
            this.action = action
        }.also { startService(it) }
    }
}

// ── Composable privé : onglet pill ────────────────────────────────────────────

@Composable
private fun TabChip(
    label: String,
    icon: ImageVector,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        Modifier
            .clip(RoundedCornerShape(20.dp))
            .background(if (selected) Color(0x44FFFFFF) else Color(0x22FFFFFF))
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            modifier = Modifier.size(15.dp),
            tint = if (selected) Color.White else Color.White.copy(alpha = 0.55f),
        )
        Text(
            label,
            fontSize = 13.sp,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
            color = if (selected) Color.White else Color.White.copy(alpha = 0.55f),
        )
    }
}
