package com.videoindiren.app

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.yausername.ffmpeg.FFmpeg
import com.yausername.youtubedl_android.YoutubeDL
import kotlin.concurrent.thread

@TauriPlugin
class ToolsPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun prepare(invoke: Invoke) {
    val context = activity.applicationContext
    thread {
      try {
        YoutubeDL.getInstance().init(context)
        FFmpeg.getInstance().init(context)
        val dirs = JSObject()
        dirs.put("nativeLibDir", context.applicationInfo.nativeLibraryDir)
        dirs.put("noBackupDir", context.noBackupFilesDir.absolutePath)
        dirs.put("cacheDir", context.cacheDir.absolutePath)
        invoke.resolve(dirs)
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }
}
