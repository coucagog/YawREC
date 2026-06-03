package app.yawrec.mobile.recording

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import app.yawrec.mobile.RecordingState
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.sqrt

class AudioCapturer(
    private val projection: MediaProjection,
    private val muxer: MediaMuxerWrapper,
) {
    private val stopped = AtomicBoolean(false)
    private val paused  = AtomicBoolean(false)

    // System audio (AudioPlaybackCapture)
    private var sysRecord: AudioRecord? = null
    // Microphone (optional, toggled at runtime)
    @Volatile private var micRecord: AudioRecord? = null
    private val micActive = AtomicBoolean(false)

    private var encoder: MediaCodec? = null
    private var audioTrackIndex = -1
    private var captureThread: Thread? = null

    companion object {
        private const val SAMPLE_RATE   = 44_100
        private const val CHANNEL_COUNT = 2
        private const val CHANNEL_MASK  = AudioFormat.CHANNEL_IN_STEREO
        private const val PCM_ENCODING  = AudioFormat.ENCODING_PCM_16BIT
        private const val BIT_RATE      = 128_000
        // 1024 samples/channel × 2 ch → one AAC-LC frame at 44100 Hz ≈ 23 ms
        private const val FRAME_SHORTS  = 1024 * CHANNEL_COUNT
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    fun start() {
        sysRecord = buildSysRecord().also { it.startRecording() }

        val aacFormat = MediaFormat.createAudioFormat(
            MediaFormat.MIMETYPE_AUDIO_AAC, SAMPLE_RATE, CHANNEL_COUNT
        ).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, BIT_RATE)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, FRAME_SHORTS * 2 * 2)
        }
        encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).also { codec ->
            codec.configure(aacFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            codec.start()
        }

        captureThread = Thread(::captureLoop, "yawrec-audio-encode").also { it.start() }
    }

    fun pause()  = paused.set(true)
    fun resume() = paused.set(false)

    /**
     * Toggle microphone mixing at runtime.
     * Safe to call while captureLoop is running.
     */
    fun setMicEnabled(enabled: Boolean) {
        if (enabled) {
            if (micRecord == null) {
                val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_MASK, PCM_ENCODING)
                micRecord = AudioRecord(
                    MediaRecorder.AudioSource.MIC,
                    SAMPLE_RATE, CHANNEL_MASK, PCM_ENCODING,
                    maxOf(minBuf, FRAME_SHORTS * 2 * 4)
                ).also { if (it.state == AudioRecord.STATE_INITIALIZED) it.startRecording() }
            }
            micActive.set(true)
        } else {
            micActive.set(false)       // captureLoop stops reading mic on next iteration
            val r = micRecord
            micRecord = null           // null before release so captureLoop safe-call returns
            r?.apply { stop(); release() }
        }
    }

    fun stop() {
        stopped.set(true)
        captureThread?.join(5_000)
        sysRecord?.apply { stop(); release() }
        sysRecord = null
        micRecord?.apply { stop(); release() }
        micRecord = null
        encoder?.apply { stop(); release() }
        encoder = null
    }

    // ── Capture / encode loop ─────────────────────────────────────────────────

    private fun captureLoop() {
        val codec    = encoder   ?: return
        val sysRec   = sysRecord ?: return
        val outInfo  = MediaCodec.BufferInfo()
        val sysBuf   = ShortArray(FRAME_SHORTS)
        val micBuf   = ShortArray(FRAME_SHORTS)
        var ptsUs    = 0L
        var eosSent  = false

        loop@ while (true) {

            // Pause: drain ring buffers without encoding
            while (paused.get() && !stopped.get()) {
                sysRec.read(sysBuf, 0, sysBuf.size, AudioRecord.READ_NON_BLOCKING)
                micRecord?.read(micBuf, 0, micBuf.size, AudioRecord.READ_NON_BLOCKING)
                Thread.sleep(10)
            }

            // ── Feed encoder input ────────────────────────────────────────────
            if (!eosSent) {
                val inputIdx = codec.dequeueInputBuffer(5_000L)
                if (inputIdx >= 0) {
                    val inputBuf = codec.getInputBuffer(inputIdx)!!
                    inputBuf.clear()

                    if (stopped.get()) {
                        codec.queueInputBuffer(inputIdx, 0, 0, ptsUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                        eosSent = true
                    } else {
                        val sysRead = sysRec.read(sysBuf, 0, sysBuf.size, AudioRecord.READ_NON_BLOCKING)
                        if (sysRead > 0) {
                            // Optionally mix microphone
                            if (micActive.get()) {
                                val micRead = micRecord?.read(micBuf, 0, sysRead, AudioRecord.READ_NON_BLOCKING) ?: 0
                                mixInPlace(sysBuf, micBuf, sysRead, micRead)
                            }
                            inputBuf.order(ByteOrder.nativeOrder())
                            inputBuf.asShortBuffer().put(sysBuf, 0, sysRead)
                            codec.queueInputBuffer(inputIdx, 0, sysRead * 2, ptsUs, 0)
                            ptsUs += sysRead.toLong() * 1_000_000L / (SAMPLE_RATE.toLong() * CHANNEL_COUNT)
                            RecordingState.updateVuLevel(rms(sysBuf, sysRead))
                        } else {
                            codec.queueInputBuffer(inputIdx, 0, 0, ptsUs, 0)
                            Thread.sleep(2)
                        }
                    }
                }
            }

            // ── Drain encoder output ──────────────────────────────────────────
            var outputIdx = codec.dequeueOutputBuffer(outInfo, 0L)
            while (outputIdx != MediaCodec.INFO_TRY_AGAIN_LATER) {
                when {
                    outputIdx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        audioTrackIndex = muxer.addTrack(codec.outputFormat)
                    }
                    outputIdx >= 0 -> {
                        val isConfig = outInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                        if (!isConfig && audioTrackIndex >= 0 && outInfo.size > 0) {
                            codec.getOutputBuffer(outputIdx)?.let { buf ->
                                muxer.writeSampleData(audioTrackIndex, buf, outInfo)
                            }
                        }
                        codec.releaseOutputBuffer(outputIdx, false)
                        if (outInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) break@loop
                    }
                }
                outputIdx = codec.dequeueOutputBuffer(outInfo, 0L)
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private fun buildSysRecord(): AudioRecord {
        val captureConfig = AudioPlaybackCaptureConfiguration.Builder(projection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_MASK, PCM_ENCODING)
        return AudioRecord.Builder()
            .setAudioPlaybackCaptureConfig(captureConfig)
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(PCM_ENCODING)
                    .setSampleRate(SAMPLE_RATE)
                    .setChannelMask(CHANNEL_MASK)
                    .build()
            )
            .setBufferSizeInBytes(maxOf(minBuf, FRAME_SHORTS * 2 * 4))
            .build()
    }

    /**
     * Mix mic into sys in place using clamped sum.
     * Only mixes up to `micRead` samples; remainder stays as sys-only.
     */
    private fun mixInPlace(sys: ShortArray, mic: ShortArray, sysCount: Int, micCount: Int) {
        val limit = minOf(sysCount, micCount)
        for (i in 0 until limit) {
            val sum = sys[i].toInt() + mic[i].toInt()
            sys[i] = sum.coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()
        }
    }

    /** RMS over `count` interleaved 16-bit PCM samples, normalised to 0..1 */
    private fun rms(buf: ShortArray, count: Int): Float {
        if (count <= 0) return 0f
        var sumSq = 0.0
        for (i in 0 until count) {
            val s = buf[i].toDouble()
            sumSq += s * s
        }
        return (sqrt(sumSq / count) / Short.MAX_VALUE).toFloat()
    }
}
