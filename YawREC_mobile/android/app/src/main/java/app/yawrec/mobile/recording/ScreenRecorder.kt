package app.yawrec.mobile.recording

import android.content.Context
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.Looper
import android.view.Surface
import java.util.concurrent.atomic.AtomicBoolean

class ScreenRecorder(
    context: Context,
    private val projection: MediaProjection,
    private val muxer: MediaMuxerWrapper,
    private val clock: PauseClock,
    /** Appelé (thread principal) si l'utilisateur coupe le partage depuis le système. */
    private val onProjectionStopped: () -> Unit,
) {
    private val stopped = AtomicBoolean(false)

    private val metrics       = context.resources.displayMetrics
    private val screenWidth   = (metrics.widthPixels  / 16) * 16   // H.264 alignment
    private val screenHeight  = (metrics.heightPixels / 16) * 16
    private val screenDensity = metrics.densityDpi

    private var virtualDisplay: VirtualDisplay? = null
    private var inputSurface: Surface? = null
    private var encoder: MediaCodec? = null
    private var videoTrackIndex = -1
    private var drainThread: Thread? = null

    fun start() {
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, screenWidth, screenHeight).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, 8_000_000)
            setInteger(MediaFormat.KEY_FRAME_RATE, 30)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
            setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
            setInteger(MediaFormat.KEY_LEVEL,   MediaCodecInfo.CodecProfileLevel.AVCLevel41)
        }

        val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        val surface = codec.createInputSurface()
        codec.start()
        encoder = codec
        inputSurface = surface

        // Android 14+ requires a callback registered before createVirtualDisplay()
        projection.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() { onProjectionStopped() }
        }, Handler(Looper.getMainLooper()))

        virtualDisplay = projection.createVirtualDisplay(
            "YawREC-screen",
            screenWidth, screenHeight, screenDensity,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            surface,
            null, null
        )

        drainThread = Thread(::drainLoop, "yawrec-video-drain").also { it.start() }
    }

    private fun drainLoop() {
        val info  = MediaCodec.BufferInfo()
        val codec = encoder ?: return
        var lastPtsUs = -1L
        var idleAfterStop = 0

        try { loop@ while (true) {
            when (val idx = codec.dequeueOutputBuffer(info, 10_000L)) {
                MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    videoTrackIndex = muxer.addTrack(codec.outputFormat)
                }
                MediaCodec.INFO_TRY_AGAIN_LATER -> {
                    // Après l'arrêt, on attend l'EOS au plus ~2 s.
                    if (stopped.get() && ++idleAfterStop > 200) break@loop
                }
                else -> if (idx >= 0) {
                    val isConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    // Timestamps Surface (CLOCK_MONOTONIC) → temps média sans les pauses.
                    val mediaPts = clock.toMediaUs(info.presentationTimeUs)
                    if (!isConfig && videoTrackIndex >= 0 && info.size > 0 &&
                        mediaPts != null && mediaPts > lastPtsUs) {
                        info.presentationTimeUs = mediaPts
                        lastPtsUs = mediaPts
                        codec.getOutputBuffer(idx)?.let { buf ->
                            muxer.writeSampleData(videoTrackIndex, buf, info)
                        }
                    }
                    codec.releaseOutputBuffer(idx, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break@loop
                }
            }
        } } catch (e: IllegalStateException) {
            // Encodeur libéré pendant la boucle (arrêt forcé) : rien à faire.
        }
    }

    /**
     * Pause réelle : le VirtualDisplay est détaché de l'encodeur, plus aucune
     * image n'est produite (avant, l'encodeur continuait de recevoir les
     * images et elles réapparaissaient à la reprise).
     */
    fun pause() {
        virtualDisplay?.surface = null
    }

    fun resume() {
        virtualDisplay?.surface = inputSurface
    }

    fun stop() {
        if (!stopped.compareAndSet(false, true)) return
        try { encoder?.signalEndOfInputStream() } catch (_: Exception) {}
        drainThread?.join(4_000)
        virtualDisplay?.release()
        virtualDisplay = null
        encoder?.apply {
            try { stop() } catch (_: Exception) {}
            release()
        }
        encoder = null
        inputSurface?.release()
        inputSurface = null
    }
}
