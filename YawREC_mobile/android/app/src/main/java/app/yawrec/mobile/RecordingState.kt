package app.yawrec.mobile

import android.os.SystemClock
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File

enum class RecordingPhase { Idle, Recording, Paused }

object RecordingState {

    private val _phase        = MutableStateFlow(RecordingPhase.Idle)
    private val _elapsedMs    = MutableStateFlow(0L)
    private val _fileSizeBytes = MutableStateFlow(0L)

    private val _vuLevel = MutableStateFlow(0f)

    val phase:         StateFlow<RecordingPhase> = _phase.asStateFlow()
    val elapsedMs:     StateFlow<Long>           = _elapsedMs.asStateFlow()
    val fileSizeBytes: StateFlow<Long>           = _fileSizeBytes.asStateFlow()
    val vuLevel:       StateFlow<Float>          = _vuLevel.asStateFlow()

    // time tracking (written from service thread only, read from timer coroutine)
    @Volatile private var startElapsedMs      = 0L
    @Volatile private var pausedTotalMs       = 0L
    @Volatile private var pauseStartElapsedMs = 0L

    fun onStart() {
        startElapsedMs      = SystemClock.elapsedRealtime()
        pausedTotalMs       = 0L
        pauseStartElapsedMs = 0L
        _elapsedMs.value    = 0L
        _fileSizeBytes.value = 0L
        _phase.value        = RecordingPhase.Recording
    }

    fun onPause() {
        pauseStartElapsedMs = SystemClock.elapsedRealtime()
        _phase.value = RecordingPhase.Paused
    }

    fun onResume() {
        val now = SystemClock.elapsedRealtime()
        if (pauseStartElapsedMs > 0L) {
            pausedTotalMs      += now - pauseStartElapsedMs
            pauseStartElapsedMs = 0L
        }
        _phase.value = RecordingPhase.Recording
    }

    fun updateVuLevel(rms: Float) {
        _vuLevel.value = rms.coerceIn(0f, 1f)
    }

    fun onStop() {
        _phase.value        = RecordingPhase.Idle
        _elapsedMs.value    = 0L
        _fileSizeBytes.value = 0L
        _vuLevel.value      = 0f
        startElapsedMs      = 0L
        pausedTotalMs       = 0L
        pauseStartElapsedMs = 0L
    }

    // called every 250 ms by the timer coroutine, only when Recording
    fun tick(outputFile: File) {
        val now = SystemClock.elapsedRealtime()
        _elapsedMs.value    = now - startElapsedMs - pausedTotalMs
        _fileSizeBytes.value = outputFile.length()
    }
}
