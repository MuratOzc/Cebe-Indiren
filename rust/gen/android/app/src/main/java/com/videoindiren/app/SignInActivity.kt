package com.videoindiren.app

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.io.File

class SignInActivity : AppCompatActivity() {
  private val hosts = linkedSetOf<String>()
  private lateinit var webView: WebView

  @SuppressLint("SetJavaScriptEnabled")
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    val target = Uri.parse(intent.getStringExtra(EXTRA_URL) ?: "")
    target.host?.let { hosts.add(it) }

    webView = WebView(this)
    val cookies = CookieManager.getInstance()
    cookies.setAcceptCookie(true)
    cookies.setAcceptThirdPartyCookies(webView, true)
    webView.settings.apply {
      javaScriptEnabled = true
      domStorageEnabled = true
      // Google refuses sign-in from user agents that admit to being a WebView.
      userAgentString = userAgentString.replace("; wv", "")
    }
    webView.webViewClient = object : WebViewClient() {
      override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
        request.url.host?.let { hosts.add(it) }
        return false
      }

      override fun onPageFinished(view: WebView, url: String) {
        Uri.parse(url).host?.let { hosts.add(it) }
      }
    }

    val title = TextView(this).apply {
      text = "Giriş yapın, sonra Bitti'ye dokunun"
      setTextSize(TypedValue.COMPLEX_UNIT_SP, 15f)
      setTextColor(Color.BLACK)
      layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
    }
    val done = Button(this).apply {
      text = "Bitti"
      isAllCaps = false
      setOnClickListener { finishWithCookies() }
    }
    val bar = LinearLayout(this).apply {
      orientation = LinearLayout.HORIZONTAL
      gravity = Gravity.CENTER_VERTICAL
      setBackgroundColor(Color.WHITE)
      val pad = (12 * resources.displayMetrics.density).toInt()
      setPadding(pad, pad / 2, pad / 2, pad / 2)
      addView(title)
      addView(done)
    }
    val root = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      setBackgroundColor(Color.WHITE)
      addView(bar)
      addView(webView, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
    }
    ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    setContentView(root)

    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        if (webView.canGoBack()) webView.goBack() else finishWithCookies()
      }
    })

    webView.loadUrl(startUrl(target))
  }

  private fun finishWithCookies() {
    CookieManager.getInstance().flush()
    val path = intent.getStringExtra(EXTRA_PATH)
    val saved = path != null && exportCookies(hosts, File(path)) > 0
    setResult(if (saved) Activity.RESULT_OK else Activity.RESULT_CANCELED)
    finish()
  }

  override fun onDestroy() {
    webView.destroy()
    super.onDestroy()
  }

  companion object {
    const val EXTRA_URL = "url"
    const val EXTRA_PATH = "path"
    private const val YEAR_SECONDS = 365L * 24 * 60 * 60
    private const val HTTP_ONLY = "#HttpOnly_"

    private fun isYouTube(host: String) =
      host == "youtu.be" || host == "youtube.com" || host.endsWith(".youtube.com")

    private fun startUrl(target: Uri): String {
      val host = target.host ?: return "https://accounts.google.com/"
      if (isYouTube(host)) {
        return "https://accounts.google.com/ServiceLogin?service=youtube&continue=" +
          Uri.encode("https://m.youtube.com/")
      }
      return "https://$host/"
    }

    // Each host is exported under every parent domain so cookies set for m.youtube.com's
    // parent also reach www.youtube.com, where yt-dlp asks for them.
    fun exportCookies(hosts: Set<String>, file: File): Int {
      val domains = hosts.flatMap { host ->
        val labels = host.lowercase().trimStart('.').split('.')
        (0 until labels.size - 1).map { labels.drop(it).joinToString(".") }
      }.distinct().sortedBy { it.length }

      val manager = CookieManager.getInstance()
      val expiry = System.currentTimeMillis() / 1000 + YEAR_SECONDS
      val seen = HashSet<String>()
      val lines = mutableListOf<String>()
      for (domain in domains) {
        val raw = manager.getCookie("https://$domain") ?: continue
        for (part in raw.split(';')) {
          val pair = part.trim()
          val split = pair.indexOf('=')
          if (split <= 0 || !seen.add(pair)) continue
          val name = pair.substring(0, split)
          val value = pair.substring(split + 1)
          lines.add(".$domain\tTRUE\t/\tTRUE\t$expiry\t$name\t$value")
        }
      }
      if (lines.isEmpty()) return 0

      val replaced = domains.toSet()
      val kept = if (file.isFile) {
        file.readLines().filter { line ->
          val cookie = line.removePrefix(HTTP_ONLY)
          val domain = cookie.substringBefore('\t').trimStart('.')
          !cookie.startsWith("#") && cookie.isNotBlank() && domain !in replaced
        }
      } else {
        emptyList()
      }
      file.parentFile?.mkdirs()
      file.writeText((listOf("# Netscape HTTP Cookie File") + kept + lines).joinToString("\n", postfix = "\n"))
      return lines.size
    }
  }
}
