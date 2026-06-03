package app.yawrec.mobile

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

object RecordingConfig {

    private val _micEnabled    = MutableStateFlow(false)
    private val _cameraEnabled = MutableStateFlow(false)

    val micEnabled:    StateFlow<Boolean> = _micEnabled.asStateFlow()
    val cameraEnabled: StateFlow<Boolean> = _cameraEnabled.asStateFlow()

    fun setMic(enabled: Boolean)    { _micEnabled.value    = enabled }
    fun setCamera(enabled: Boolean) { _cameraEnabled.value = enabled }

    fun reset() {
        _micEnabled.value    = false
        _cameraEnabled.value = false
    }
}
