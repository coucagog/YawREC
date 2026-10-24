package app.yawrec.mobile.recording

/**
 * Horloge média partagée par la vidéo et l'audio d'un enregistrement.
 *
 * Les deux pistes sont datées en microsecondes sur CLOCK_MONOTONIC
 * (System.nanoTime / 1000 — c'est aussi l'horloge des timestamps de la
 * Surface du VirtualDisplay). Cette classe retire de ces instants la durée
 * des pauses qui les précèdent, pour que vidéo et audio restent alignées et
 * continues après une pause.
 */
class PauseClock {

    // Intervalles [début, fin] en µs ; fin = -1 tant que la pause est en cours.
    private val intervals = mutableListOf<LongArray>()

    @Synchronized
    fun pause(nowUs: Long = nowUs()) {
        if (intervals.lastOrNull()?.get(1) == -1L) return
        intervals.add(longArrayOf(nowUs, -1L))
    }

    @Synchronized
    fun resume(nowUs: Long = nowUs()) {
        val last = intervals.lastOrNull() ?: return
        if (last[1] == -1L) last[1] = maxOf(nowUs, last[0])
    }

    /** Instant mur → instant média, ou null s'il tombe pendant une pause. */
    @Synchronized
    fun toMediaUs(wallUs: Long): Long? {
        var paused = 0L
        for (iv in intervals) {
            if (wallUs < iv[0]) break
            if (iv[1] >= 0 && wallUs >= iv[1]) paused += iv[1] - iv[0] else return null
        }
        return wallUs - paused
    }

    companion object {
        fun nowUs(): Long = System.nanoTime() / 1_000
    }
}
