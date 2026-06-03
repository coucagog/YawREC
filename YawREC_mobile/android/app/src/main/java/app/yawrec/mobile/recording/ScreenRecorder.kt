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
import java.util.concurrent.atomic.AtomicBoolean

class ScreenRecorder(
    context: Context,
    private val projection: MediaProjection,
    private val muxer: MediaMuxerWrapper,
) {
    private val stopped = AtomicBoolean(false)
    private val paused  = AtomicBoolean(false)

    private val metrics       = context.resources.displayMetrics
    private val screenWidth   = (metrics.widthPixels  / 16) * 16   // H.264 alignment
    private val screenHeight  = (metrics.heightPixels / 16) * 16
    private val screenDensity = metrics.densityDpi

    private var virtualDisplay: VirtualDisplay? = null
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
        val inputSurface = codec.createInputSurface()
        codec.start()
        encoder = codec

        // Android 14+ requires a callback registered before createVirtualDisplay()
        projection.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() { stop() }
        }, Handler(Looper.getMainLooper()))

        virtualDisplay = projection.createVirtualDisplay(
            "YawREC-screen",
            screenWidth, screenHeight, screenDensity,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            inputSurface,
            null, null
        )

        drainThread = Thread(::drainLoop, "yawrec-video-drain").also { it.start() }
    }

    private fun drainLoop() {
        val info  = MediaCodec.BufferInfo()
        val codec = encoder ?: return

        loop@ while (true) {
            if (paused.get() && !stopped.get()) {
                Thread.sleep(16)
                continue
            }

            when (val idx = codec.dequeueOutputBuffer(info, 10_000L)) {
                MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    videoTrackIndex = muxer.addTrack(codec.outputFormat)
                }
                MediaCodec.INFO_TRY_AGAIN_LATER -> {
                    if (stopped.get()) break@loop   // encoder drained, exit
                }
                else -> if (idx >= 0) {
                    val isConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                    if (!isConfig && videoTrackIndex >= 0 && info.size > 0) {
                        codec.getOutputBuffer(idx)?.let { buf ->
                            muxer.writeSampleData(videoTrackIndex, buf, info)
                        }
                    }
                    codec.releaseOutputBuffer(idx, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break@loop
                }
            }
        }
    }

    fun pause()  = paused.set(true)
    fun resume() = paused.set(false)

    fun stop() {
        stopped.set(true)
        try { encoder?.signalEndOfInputStream() } catch (_: Exception) {}
        drainThread?.join(4_000)
        virtualDisplay?.release()
        encoder?.apply { stop(); release() }
        encoder = null
    }
}
