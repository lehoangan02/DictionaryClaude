package dev.lexis.gre

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File
import android.view.View
import android.media.MediaPlayer
import android.speech.tts.TextToSpeech
import android.util.Base64
import android.webkit.JavascriptInterface
import android.webkit.WebView
import java.util.Locale
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Keep the page clear of the status bar, notch and gesture bar.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { v, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      // The on-screen keyboard: shrink the page by its height so the message box stays visible above it.
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
      v.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard))
      WindowInsetsCompat.CONSUMED
    }
  }

  private var tts: TextToSpeech? = null
  private var player: MediaPlayer? = null

  /** The page asks for a spoken word here when no recording can be played. */
  inner class Voice {
    @JavascriptInterface
    fun speak(word: String) {
      tts?.speak(word, TextToSpeech.QUEUE_FLUSH, null, "word")
    }

    /**
     * Plays a recording (base64 MP3/OGG) with the system player: the web view refuses some audio data URLs
     * ("no supported source was found"). Returns false when it could not play, so the page can try its own player.
     */
    @JavascriptInterface
    fun play(base64: String): Boolean {
      return try {
        player?.release()
        val f = File(cacheDir, "word.audio")
        f.writeBytes(Base64.decode(base64, Base64.DEFAULT))
        player = MediaPlayer().apply {
          setDataSource(f.path)
          setOnCompletionListener { it.release(); if (player === it) player = null }
          prepare()
          start()
        }
        true
      } catch (e: Throwable) {
        player?.release()
        player = null
        false
      }
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    // Pronunciations play after a network fetch, so they no longer count as part of the tap.
    webView.settings.mediaPlaybackRequiresUserGesture = false
    tts = TextToSpeech(this) { status -> if (status == TextToSpeech.SUCCESS) tts?.language = Locale.US }
    webView.addJavascriptInterface(Voice(), "LadyBirdVoice")
  }

  override fun onDestroy() {
    tts?.shutdown()
    super.onDestroy()
  }

  /**
   * Hands a downloaded update to the system installer (called from Rust through JNI).
   * Returns "installer" (the system screen is opening), "permission" (the person must first allow
   * installs from LadyBird; the settings page is opening) or "error: ..." — it never throws.
   */
  fun installApk(path: String): String {
    return try {
      if (Build.VERSION.SDK_INT >= 26 && !packageManager.canRequestPackageInstalls()) {
        startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        "permission"
      } else {
        val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", File(path))
        startActivity(
          Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        )
        "installer"
      }
    } catch (e: Throwable) {
      "error: ${e.javaClass.simpleName}: ${e.message}"
    }
  }
}
