package app.yawrec.mobile.recording

import android.app.Service
import android.content.ContentValues
import android.content.Intent
import android.media.MediaMuxer
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.provider.MediaStore
import android.util.Log
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

        private const val TAG = "YawREC-Rec"
        private const val TICK_MS   = 250L
        private const val NOTIF_EVERY_N_TICKS = 4  // update notification every 1 s
    }

    /**
     * Fichier de sortie : une entrée MediaStore (Films/YawREC, visible dans la
     * galerie et conservée après désinstallation) ou, à défaut, un fichier
     * dans le dossier privé de l'app.
     */
    private class Output(val uri: Uri?, val file: File?, val pfd: ParcelFileDescriptor?) {
        fun sizeBytes(): Long = pfd?.statSize ?: file?.length() ?: 0L
    }

    private val serviceScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private var timerJob: Job? = null

    private var mediaProjection: MediaProjection? = null
    private var screenRecorder: ScreenRecorder? = null
    private var audioCapturer: AudioCapturer? = null
    private var muxerWrapper: MediaMuxerWrapper? = null
    private var pauseClock: PauseClock? = null
    private var output: Output? = null
    private var stopping = false

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
        if (muxerWrapper != null) return  // déjà en cours
        stopping = false
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

        try {
            val out = openOutput().also { output = it }
            val muxer = if (out.pfd != null) {
                MediaMuxer(out.pfd.fileDescriptor, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            } else {
                MediaMuxer(out.file!!.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            }
            // Two tracks: video (ScreenRecorder) + audio (AudioCapturer)
            val wrapper = MediaMuxerWrapper(muxer, expectedTracks = 2).also { muxerWrapper = it }
            val clock = PauseClock().also { pauseClock = it }

            val projManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
            mediaProjection = projManager.getMediaProjection(resultCode, projectionData).also { proj ->
                // Partage coupé depuis le système (bouton « Arrêter le partage ») :
                // on finalise le fichier au lieu de rester en « enregistrement ».
                screenRecorder = ScreenRecorder(this, proj, wrapper, clock, onProjectionStopped = ::handleStop)
                    .also { it.start() }
                audioCapturer = AudioCapturer(proj, wrapper, clock).also { it.start() }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Démarrage impossible", e)
            handleStop()
            return
        }

        RecordingState.onStart()
        startTimerLoop()
    }

    private fun handleStop() {
        if (stopping) return
        stopping = true

        timerJob?.cancel()
        timerJob = null

        // Stop recorders first (they flush their encoders and finish writing to muxer)
        screenRecorder?.stop()
        audioCapturer?.stop()

        // Release muxer only after both encoders have drained
        val hasData = muxerWrapper?.hasData == true
        muxerWrapper?.release()
        finalizeOutput(hasData)

        mediaProjection?.stop()

        screenRecorder  = null
        audioCapturer   = null
        muxerWrapper    = null
        mediaProjection = null
        pauseClock      = null

        RecordingState.onStop()
        RecordingConfig.reset()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun handlePause() {
        if (RecordingState.phase.value != RecordingPhase.Recording) return
        pauseClock?.pause()
        screenRecorder?.pause()
        RecordingState.onPause()
        notification.update(RecordingState.elapsedMs.value, isPaused = true)
    }

    private fun handleResume() {
        if (RecordingState.phase.value != RecordingPhase.Paused) return
        screenRecorder?.resume()
        pauseClock?.resume()
        RecordingState.onResume()
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

    private fun startTimerLoop() {
        timerJob = serviceScope.launch {
            var tick = 0
            while (isActive) {
                delay(TICK_MS)
                if (RecordingState.phase.value == RecordingPhase.Recording) {
                    RecordingState.tick(output?.sizeBytes() ?: 0L)
                    if (++tick % NOTIF_EVERY_N_TICKS == 0) {
                        notification.update(RecordingState.elapsedMs.value, isPaused = false)
                    }
                }
            }
        }
    }

    // ── Fichier de sortie ─────────────────────────────────────────────────────

    private fun openOutput(): Output {
        val name = "YawREC_" + SimpleDateFormat("yyyy-MM-dd_HH-mm-ss", Locale.US).format(Date()) + ".mp4"
        try {
            val values = ContentValues().apply {
                put(MediaStore.Video.Media.DISPLAY_NAME, name)
                put(MediaStore.Video.Media.MIME_TYPE, "video/mp4")
                put(MediaStore.Video.Media.RELATIVE_PATH, Environment.DIRECTORY_MOVIES + "/YawREC")
                put(MediaStore.Video.Media.IS_PENDING, 1)
            }
            val uri = contentResolver.insert(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, values)
            val pfd = uri?.let { contentResolver.openFileDescriptor(it, "rw") }
            if (uri != null && pfd != null) return Output(uri, null, pfd)
            uri?.let { contentResolver.delete(it, null, null) }
        } catch (e: Exception) {
            Log.w(TAG, "MediaStore indisponible, repli sur le dossier de l'app", e)
        }
        val dir = File(getExternalFilesDir(Environment.DIRECTORY_MOVIES), "YawREC").also { it.mkdirs() }
        return Output(null, File(dir, name), null)
    }

    private fun finalizeOutput(hasData: Boolean) {
        val out = output ?: return
        output = null
        try { out.pfd?.close() } catch (_: Exception) {}
        val uri = out.uri
        if (uri != null) {
            if (hasData) {
                contentResolver.update(uri, ContentValues().apply {
                    put(MediaStore.Video.Media.IS_PENDING, 0)
                }, null, null)
            } else {
                contentResolver.delete(uri, null, null)  // enregistrement vide
            }
        } else if (!hasData) {
            out.file?.delete()
        }
    }

    override fun onDestroy() {
        handleStop()
        serviceScope.cancel()
        super.onDestroy()
    }
}
