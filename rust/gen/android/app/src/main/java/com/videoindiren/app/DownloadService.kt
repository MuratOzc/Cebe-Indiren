package com.videoindiren.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager

class DownloadService : Service() {
  private var wakeLock: PowerManager.WakeLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val notification = notification(this, percent)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
    if (wakeLock == null) {
      val power = getSystemService(Context.POWER_SERVICE) as PowerManager
      wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "VideoIndiren:download").apply {
        setReferenceCounted(false)
        acquire(MAX_WAKE_MS)
      }
    }
    instance = this
    if (!running) stopSelf()
    return START_NOT_STICKY
  }

  override fun onTimeout(startId: Int, fgsType: Int) {
    stopSelf()
  }

  override fun onDestroy() {
    wakeLock?.release()
    wakeLock = null
    if (instance === this) instance = null
    super.onDestroy()
  }

  companion object {
    private const val CHANNEL_ID = "downloads"
    private const val FINISHED_CHANNEL_ID = "finished"
    private const val NOTIFICATION_ID = 1
    private const val FINISHED_ID = 2
    private const val MAX_WAKE_MS = 6 * 60 * 60 * 1000L

    @Volatile private var running = false
    @Volatile private var percent = -1
    @Volatile private var instance: DownloadService? = null

    fun start(context: Context) {
      percent = -1
      running = true
      val intent = Intent(context, DownloadService::class.java)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        context.startForegroundService(intent)
      } else {
        context.startService(intent)
      }
    }

    fun progress(context: Context, value: Int) {
      percent = value
      if (!running) return
      val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      manager.notify(NOTIFICATION_ID, notification(context, value))
    }

    // Stopping before onStartCommand reaches startForeground crashes the app, so the service stops itself.
    fun stop() {
      running = false
      instance?.stopSelf()
    }

    fun finished(context: Context, ok: Boolean, text: String) {
      val notification = builder(context, FINISHED_CHANNEL_ID)
        .setSmallIcon(if (ok) android.R.drawable.stat_sys_download_done else android.R.drawable.stat_notify_error)
        .setContentTitle(if (ok) "İndirildi" else "İndirilemedi")
        .setContentText(text)
        .setStyle(Notification.BigTextStyle().bigText(text))
        .setAutoCancel(true)
        .build()
      val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      manager.notify(FINISHED_ID, notification)
    }

    private fun notification(context: Context, percent: Int): Notification {
      val known = percent in 0..100
      return builder(context, CHANNEL_ID)
        .setSmallIcon(android.R.drawable.stat_sys_download)
        .setContentTitle(context.getString(R.string.app_name))
        .setContentText(if (known) "İndiriliyor… %$percent" else "İndiriliyor…")
        .setProgress(100, if (known) percent else 0, !known)
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .build()
    }

    private fun builder(context: Context, channel: String): Notification.Builder {
      val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && manager.getNotificationChannel(channel) == null) {
        manager.createNotificationChannel(
          if (channel == CHANNEL_ID) {
            NotificationChannel(channel, "İndirme durumu", NotificationManager.IMPORTANCE_LOW)
          } else {
            NotificationChannel(channel, "Biten indirmeler", NotificationManager.IMPORTANCE_DEFAULT)
          }
        )
      }
      val open = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
      )
      val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        Notification.Builder(context, channel)
      } else {
        @Suppress("DEPRECATION")
        Notification.Builder(context)
      }
      return builder.setContentIntent(open)
    }
  }
}
