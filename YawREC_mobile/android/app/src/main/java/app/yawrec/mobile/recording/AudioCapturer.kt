package app.yawrec.mobile.recording

import android.annotation.SuppressLint
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

/**
 * Capture son système (AudioPlaybackCapture) + micro optionnel, encode en AAC.
 *
 * Temps : la piste est datée sur la même horloge que la vidéo (CLOCK_MONOTONIC,
 * pauses retirées via [PauseClock]). Le PTS avance au nombre d'échantillons
 * encodés ; si le son système ne livre rien pendant un moment, on comble avec
 * du silence pour que l'audio ne prenne pas de retard sur la vidéo.
 */
class AudioCapturer(
    private val projection: MediaProjection,
    private val muxer: MediaMuxerWrapper,
    private val clock: PauseClock,
) {
    private val stopped = AtomicBoolean(false)

    // System audio (AudioPlaybackCapture)
    private var sysRecord: AudioRecord? = null
    // Microphone : ouvert/fermé par le thread de capture lui-même, selon ce
    // flag, pour ne jamais libérer un AudioRecord en cours de lecture.
    private val micWanted = AtomicBoolean(false)

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
        /** Retard au-delà duquel on comble l'absence de son système par du silence. */
        private const val GAP_FILL_THRESHOLD_US = 80_000L
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

    /** Active/coupe le mixage du micro (appliqué par le thread de capture). */
    fun setMicEnabled(enabled: Boolean) = micWanted.set(enabled)

    fun stop() {
        if (!stopped.compareAndSet(false, true)) return
        captureThread?.join(5_000)
        sysRecord?.apply { stop(); release() }
        sysRecord = null
        encoder?.apply {
            try { stop() } catch (_: Exception) {}
            release()
        }
        encoder = null
    }

    // ── Capture / encode loop ─────────────────────────────────────────────────

    private fun captureLoop() {
        val codec    = encoder   ?: return
        val sysRec   = sysRecord ?: return
        val outInfo  = MediaCodec.BufferInfo()
        val sysBuf   = ShortArray(FRAME_SHORTS)
        val micBuf   = ShortArray(FRAME_SHORTS)
        var micRec: AudioRecord? = null

        // Temps média (µs) correspondant au premier échantillon encodé.
        var startMediaUs = -1L
        // Frames (par canal) envoyées à l'encodeur.
        var framesFed = 0L
        var eosSent = false

        fun ptsUs(): Long = startMediaUs + framesFed * 1_000_000L / SAMPLE_RATE

        try {
            loop@ while (true) {
                // ── Micro : ouverture / fermeture sur ce thread ───────────────
                if (micWanted.get() && micRec == null) micRec = openMic()
                if (!micWanted.get() && micRec != null) {
                    micRec.apply { stop(); release() }
                    micRec = null
                }

                // ── Feed encoder input ────────────────────────────────────────
                if (!eosSent) {
                    val sysRead = sysRec.read(sysBuf, 0, sysBuf.size, AudioRecord.READ_NON_BLOCKING)
                    val nowMedia = clock.toMediaUs(PauseClock.nowUs())

                    var count = 0
                    if (stopped.get()) {
                        // fin : on enverra l'EOS
                    } else if (nowMedia == null) {
                        // En pause : on vide les buffers sans encoder.
                        micRec?.read(micBuf, 0, micBuf.size, AudioRecord.READ_NON_BLOCKING)
                        Thread.sleep(10)
                    } else if (sysRead > 0) {
                        count = sysRead
                        if (startMediaUs < 0) {
                            startMediaUs = nowMedia - (sysRead / CHANNEL_COUNT) * 1_000_000L / SAMPLE_RATE
                        }
                    } else if (startMediaUs >= 0 && nowMedia - ptsUs() > GAP_FILL_THRESHOLD_US) {
                        // Pas de son système depuis un moment : silence.
                        sysBuf.fill(0)
                        count = sysBuf.size
                    } else {
                        Thread.sleep(2)
                    }

                    if (stopped.get() || count > 0) {
                        val inputIdx = codec.dequeueInputBuffer(10_000L)
                        if (inputIdx >= 0) {
                            if (stopped.get()) {
                                codec.queueInputBuffer(inputIdx, 0, 0, maxOf(ptsUs(), 0L), MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                                eosSent = true
                            } else {
                                // Micro lu en bloquant sur la même durée que le son
                                // système : les deux flux restent en phase (avant,
                                // une lecture non bloquante mixait des bouts de micro).
                                micRec?.let { mic ->
                                    val micRead = mic.read(micBuf, 0, count, AudioRecord.READ_BLOCKING)
                                    mixInPlace(sysBuf, micBuf, count, micRead)
                                }
                                val inputBuf = codec.getInputBuffer(inputIdx)!!
                                inputBuf.clear()
                                inputBuf.order(ByteOrder.nativeOrder())
                                inputBuf.asShortBuffer().put(sysBuf, 0, count)
                                codec.queueInputBuffer(inputIdx, 0, count * 2, ptsUs(), 0)
                                framesFed += count / CHANNEL_COUNT
                                RecordingState.updateVuLevel(rms(sysBuf, count))
                            }
                        }
                    }
                }

                // ── Drain encoder output ──────────────────────────────────────
                var outputIdx = codec.dequeueOutputBuffer(outInfo, if (eosSent) 10_000L else 0L)
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
        } catch (e: IllegalStateException) {
            // Encodeur / AudioRecord libéré pendant un arrêt forcé.
        } finally {
            micRec?.apply { try { stop() } catch (_: Exception) {}; release() }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    @SuppressLint("MissingPermission") // RECORD_AUDIO vérifiée avant le démarrage du service
    private fun openMic(): AudioRecord? {
        val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_MASK, PCM_ENCODING)
        val rec = AudioRecord(
            MediaRecorder.AudioSource.MIC,
            SAMPLE_RATE, CHANNEL_MASK, PCM_ENCODING,
            maxOf(minBuf, FRAME_SHORTS * 2 * 4)
        )
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            return null
        }
        rec.startRecording()
        return rec
    }

    @SuppressLint("MissingPermission")
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
