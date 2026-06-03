package app.yawrec.mobile

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn

sealed class RecordingUiState {
    object Idle : RecordingUiState()
    data class Recording(val elapsedMs: Long, val fileSizeBytes: Long) : RecordingUiState()
    data class Paused(val elapsedMs: Long, val fileSizeBytes: Long) : RecordingUiState()
}

class RecordingViewModel : ViewModel() {

    // Tile toggle states — driven by RecordingConfig
    val micEnabled:    StateFlow<Boolean> = RecordingConfig.micEnabled
    val cameraEnabled: StateFlow<Boolean> = RecordingConfig.cameraEnabled

    // Raw 0..1 RMS level; 0 when idle or paused
    val vuLevel: StateFlow<Float> = RecordingState.vuLevel
        .map { if (RecordingState.phase.value == RecordingPhase.Recording) it else 0f }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), 0f)

    val state: StateFlow<RecordingUiState> = combine(
        RecordingState.phase,
        RecordingState.elapsedMs,
        RecordingState.fileSizeBytes,
    ) { phase, elapsed, size ->
        when (phase) {
            RecordingPhase.Idle      -> RecordingUiState.Idle
            RecordingPhase.Recording -> RecordingUiState.Recording(elapsed, size)
            RecordingPhase.Paused    -> RecordingUiState.Paused(elapsed, size)
        }
    }.stateIn(
        scope = viewModelScope,
        started = SharingStarted.WhileSubscribed(5_000),
        initialValue = RecordingUiState.Idle
    )
}
