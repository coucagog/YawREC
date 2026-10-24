package app.yawrec.mobile.recording

import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaMuxer
import java.nio.ByteBuffer
import java.util.concurrent.TimeUnit
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

/**
 * Thread-safe MediaMuxer that auto-starts once all expected tracks have been added.
 * Both ScreenRecorder (video) and AudioCapturer (audio) call addTrack() from their own
 * threads; the second call triggers muxer.start() and unblocks any writeSampleData()
 * already waiting on the condition.
 */
class MediaMuxerWrapper(private val muxer: MediaMuxer, private val expectedTracks: Int) {

    private val lock     = ReentrantLock()
    private val ready    = lock.newCondition()

    private var added    = 0
    private var started  = false
    private var released = false

    /** true si au moins un échantillon a pu être écrit (fichier exploitable). */
    @Volatile var hasData = false
        private set

    fun addTrack(format: MediaFormat): Int = lock.withLock {
        val idx = muxer.addTrack(format)
        added++
        if (added == expectedTracks) {
            muxer.start()
            started = true
            ready.signalAll()
        }
        idx
    }

    fun writeSampleData(trackIndex: Int, buffer: ByteBuffer, info: MediaCodec.BufferInfo) {
        lock.withLock {
            var waited = 0L
            while (!started && !released && waited < 8_000L) {
                ready.await(50, TimeUnit.MILLISECONDS)
                waited += 50
            }
            if (started && !released) {
                muxer.writeSampleData(trackIndex, buffer, info)
                hasData = true
            }
        }
    }

    fun release() = lock.withLock {
        if (released) return@withLock
        released = true
        ready.signalAll()
        if (started) {
            try { muxer.stop() } catch (_: Exception) {}
        }
        try { muxer.release() } catch (_: Exception) {}
    }
}
