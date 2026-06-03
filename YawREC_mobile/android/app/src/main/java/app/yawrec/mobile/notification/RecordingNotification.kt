package app.yawrec.mobile.notification

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import app.yawrec.mobile.MainActivity
import app.yawrec.mobile.R
import app.yawrec.mobile.recording.RecordingService

class RecordingNotification(private val context: Context) {

    companion object {
        const val NOTIFICATION_ID = 1001
        private const val CHANNEL_ID = "yawrec_recording"
    }

    private val manager = context.getSystemService(NotificationManager::class.java)

    init {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Enregistrement", NotificationManager.IMPORTANCE_LOW).apply {
                description = "YawREC — enregistrement en cours"
                setShowBadge(false)
            }
        )
    }

    fun build(elapsedMs: Long, isPaused: Boolean): Notification {
        val elapsed = formatElapsed(elapsedMs)

        val stopPi = pendingServiceIntent(RecordingService.ACTION_STOP, requestCode = 0)
        val pausePi = pendingServiceIntent(
            if (isPaused) RecordingService.ACTION_RESUME else RecordingService.ACTION_PAUSE,
            requestCode = 1
        )
        val openPi = PendingIntent.getActivity(
            context, 2,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_rec)
            .setContentTitle("YawREC")
            .setContentText(if (isPaused) "En pause · $elapsed" else "Enregistrement · $elapsed")
            .setSubText(if (isPaused) null else "en cours")
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setContentIntent(openPi)
            .addAction(0, "Arrêter", stopPi)
            .addAction(0, if (isPaused) "Reprendre" else "Pause", pausePi)
            .build()
    }

    fun update(elapsedMs: Long, isPaused: Boolean) {
        manager.notify(NOTIFICATION_ID, build(elapsedMs, isPaused))
    }

    private fun pendingServiceIntent(action: String, requestCode: Int): PendingIntent =
        PendingIntent.getService(
            context, requestCode,
            Intent(context, RecordingService::class.java).apply { this.action = action },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

    private fun formatElapsed(ms: Long): String {
        val s = ms / 1000
        return "%02d:%02d:%02d".format(s / 3600, (s % 3600) / 60, s % 60)
    }
}
