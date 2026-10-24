package app.yawrec.mobile.recording

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PauseClockTest {

    @Test
    fun sansPause_tempsInchange() {
        val c = PauseClock()
        assertEquals(1_000L, c.toMediaUs(1_000L))
    }

    @Test
    fun pausesRetireesDuTempsMedia() {
        val c = PauseClock()
        c.pause(1_000L)
        assertNull(c.toMediaUs(1_500L))
        c.resume(4_000L)
        assertEquals(999L, c.toMediaUs(999L))
        assertEquals(1_000L, c.toMediaUs(4_000L))
        assertEquals(2_000L, c.toMediaUs(5_000L))
        c.pause(6_000L)
        c.resume(6_500L)
        assertEquals(3_500L, c.toMediaUs(7_000L))
    }

    @Test
    fun doublePauseIgnoree() {
        val c = PauseClock()
        c.pause(100L)
        c.pause(200L)
        c.resume(300L)
        c.resume(400L)
        assertEquals(300L, c.toMediaUs(500L))
    }
}
