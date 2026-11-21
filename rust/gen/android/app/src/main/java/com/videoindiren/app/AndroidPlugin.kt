package com.videoindiren.app

import android.Manifest
import android.app.Activity
import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.DocumentsContract
import android.webkit.MimeTypeMap
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.yausername.ffmpeg.FFmpeg
import com.yausername.youtubedl_android.YoutubeDL
import java.io.File
import kotlin.concurrent.thread

@InvokeArg
class PathArgs {
  var path: String = ""
}

@InvokeArg
class PathsArgs {
  var paths: Array<String> = emptyArray()
}

@TauriPlugin
class AndroidPlugin(private val activity: Activity) : Plugin(activity) {
  override fun load(webView: android.webkit.WebView) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q &&
      activity.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) != PackageManager.PERMISSION_GRANTED
    ) {
      activity.requestPermissions(arrayOf(Manifest.permission.WRITE_EXTERNAL_STORAGE), 1)
    }
  }

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

  @Command
  fun openFile(invoke: Invoke) {
    val file = File(invoke.parseArgs(PathArgs::class.java).path)
    val extension = file.extension.lowercase()
    val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension) ?: "*/*"
    try {
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      val intent = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, mime)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      activity.startActivity(Intent.createChooser(intent, null))
      invoke.resolve()
    } catch (e: ActivityNotFoundException) {
      invoke.reject("Bu dosyayı açabilecek bir uygulama yok.")
    } catch (e: IllegalArgumentException) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  @Command
  fun scanPaths(invoke: Invoke) {
    MediaScannerConnection.scanFile(activity.applicationContext, invoke.parseArgs(PathsArgs::class.java).paths, null, null)
    invoke.resolve()
  }

  @Command
  fun showDownloads(invoke: Invoke) {
    try {
      activity.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS))
      invoke.resolve()
    } catch (e: ActivityNotFoundException) {
      invoke.reject("İndirilenler klasörü açılamadı.")
    }
  }

  @Command
  fun pickFolder(invoke: Invoke) {
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val initial = treeDocumentId(invoke.parseArgs(PathArgs::class.java).path) ?: "primary:${Environment.DIRECTORY_DOWNLOADS}"
      intent.putExtra(
        DocumentsContract.EXTRA_INITIAL_URI,
        DocumentsContract.buildDocumentUri(EXTERNAL_STORAGE_AUTHORITY, initial)
      )
    }
    startActivityForResult(invoke, intent, "folderPicked")
  }

  @ActivityCallback
  fun folderPicked(invoke: Invoke, result: ActivityResult) {
    val uri: Uri? = result.data?.data
    val picked = JSObject()
    if (result.resultCode != Activity.RESULT_OK || uri == null) {
      invoke.resolve(picked)
      return
    }
    val path = downloadsPath(DocumentsContract.getTreeDocumentId(uri))
    if (path == null) {
      invoke.reject("Android'de yalnızca İndirilenler (Download) klasörü ve alt klasörleri seçilebilir.")
      return
    }
    picked.put("path", path)
    invoke.resolve(picked)
  }

  // yt-dlp writes by file path, which shared storage only allows under Download.
  private fun downloadsPath(documentId: String): String? {
    val (volume, relative) = documentId.split(":", limit = 2).let { it[0] to it.getOrElse(1) { "" } }
    val downloads = Environment.DIRECTORY_DOWNLOADS
    if (volume != "primary" || !(relative == downloads || relative.startsWith("$downloads/"))) return null
    return File(Environment.getExternalStorageDirectory(), relative).absolutePath
  }

  private fun treeDocumentId(path: String): String? {
    val root = Environment.getExternalStorageDirectory().absolutePath
    val canonical = try { File(path).canonicalPath } catch (e: Exception) { return null }
    val rootCanonical = File(root).canonicalPath
    if (!canonical.startsWith("$rootCanonical/")) return null
    return "primary:" + canonical.removePrefix("$rootCanonical/")
  }

  companion object {
    private const val EXTERNAL_STORAGE_AUTHORITY = "com.android.externalstorage.documents"
  }
}
