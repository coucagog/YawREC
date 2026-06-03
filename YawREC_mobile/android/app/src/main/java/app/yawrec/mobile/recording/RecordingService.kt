package app.yawrec.mobile.recording

import android.app.Service
import android.content.Intent
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Environment
import android.os.IBinder
import app.yawrec.mobile.RecordingConfig
import app.yawrec.mobile.RecordingPhase
import app.yawrec.mobile.RecordingState
import app.yawrec.mobile.notification.RecordingNotification
import kotlinx.coroutines.*
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class RecordingService : Service() {

    companion object {
        const val ACTION_START         = "app.yawrec.mobile.START"
        const val ACTION_STOP          = "app.yawrec.mobile.STOP"
        const val ACTION_PAUSE         = "app.yawrec.mobile.PAUSE"
        const val ACTION_RESUME        = "app.yawrec.mobile.RESUME"
        const val ACTION_TOGGLE_MIC    = "app.yawrec.mobile.TOGGLE_MIC"
        const val ACTION_TOGGLE_CAMERA = "app.yawrec.mobile.TOGGLE_CAMERA"
        const val EXTRA_RESULT_CODE     = "extra.RESULT_CODE"
        const val EXTRA_PROJECTION_DATA = "extra.PROJECTION_DATA"

        private const val TICK_MS   = 250L
        private const val NOTIF_EVERY_N_TICKS = 4  // update notification every 1 s
    }

    private val serviceScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private var timerJob: Job? = null

    private var mediaProjection: MediaProjection? = null
    private var screenRecorder: ScreenRecorder? = null
    private var audioCapturer: AudioCapturer? = null
    private var muxerWrapper: MediaMuxerWrapper? = null
    private var outputFile: File? = null

    private lateinit var notification: RecordingNotification

    override fun onCreate() {
        super.onCreate()
        notification = RecordingNotification(this)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START         -> handleStart(intent)
            ACTION_STOP          -> handleStop()
            ACTION_PAUSE         -> handlePause()
            ACTION_RESUME        -> handleResume()
            ACTION_TOGGLE_MIC    -> handleToggleMic()
            ACTION_TOGGLE_CAMERA -> handleToggleCamera()
        }
        return START_NOT_STICKY
    }

    // ── Handlers ─────────────────────────────────────────────────────────────

    private fun handleStart(intent: Intent) {
        val resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, 0)
        val projectionData: Intent? = if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableExtra(EXTRA_PROJECTION_DATA, Intent::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra(EXTRA_PROJECTION_DATA)
        }
        if (projectionData == null) { stopSelf(); return }

        startForeground(
            RecordingNotification.NOTIFICATION_ID,
            notification.build(0L, isPaused = false)
        )

        RecordingState.onStart()

        val file = makeOutputFile().also { outputFile = it }
        // Two tracks: video (ScreenRecorder) + audio (AudioCapturer)
        val wrapper = MediaMuxerWrapper(file.absolutePath, expectedTracks = 2)
            .also { muxerWrapper = it }

        val projManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        mediaProjection = projManager.getMediaProjection(resultCode, projectionData).also { proj ->
            screenRecorder = ScreenRecorder(this, proj, wrapper).also { it.start() }
            audioCapturer  = AudioCapturer(proj, wrapper).also { it.start() }
        }

        startTimerLoop(file)
    }

    private fun handleStop() {
        timerJob?.cancel()
        timerJob = null

        // Stop recorders first (they flush their encoders and finish writing to muxer)
        screenRecorder?.stop()
        audioCapturer?.stop()

        // Release muxer only after both encoders have drained
        muxerWrapper?.release()

        mediaProjection?.stop()

        screenRecorder  = null
        audioCapturer   = null
        muxerWrapper    = null
        mediaProjection = null

        RecordingState.onStop()
        RecordingConfig.reset()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun handlePause() {
        RecordingState.onPause()
        screenRecorder?.pause()
        audioCapturer?.pause()
        notification.update(RecordingState.elapsedMs.value, isPaused = true)
    }

    private fun handleResume() {
        RecordingState.onResume()
        screenRecorder?.resume()
        audioCapturer?.resume()
        // Notification will be updated by the next timer tick
    }

    private fun handleToggleMic() {
        val next = !RecordingConfig.micEnabled.value
        RecordingConfig.setMic(next)
        audioCapturer?.setMicEnabled(next)
    }

    private fun handleToggleCamera() {
        RecordingConfig.setCamera(!RecordingConfig.cameraEnabled.value)
        // Camera PiP compositing: future sprint
    }

    // ── Timer loop ────────────────────────────────────────────────────────────

    private fun startTimerLoop(file: File) {
        timerJob = serviceScope.launch {
            var tick = 0
            while (isActive) {
                delay(TICK_MS)
                if (RecordingState.phase.value == RecordingPhase.Recording) {
                    RecordingState.tick(file)
                    if (++tick % NOTIF_EVERY_N_TICKS == 0) {
                        notification.update(RecordingState.elapsedMs.value, isPaused = false)
                    }
                }
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private fun makeOutputFile(): File {
        val dir = File(getExternalFilesDir(Environment.DIRECTORY_MOVIES), "YawREC")
            .also { it.mkdirs() }
        val ts = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
        return File(dir, "YawREC_$ts.mp4")
    }

    override fun onDestroy() {
        serviceScope.cancel()
        handleStop()
        super.onDestroy()
    }
}
