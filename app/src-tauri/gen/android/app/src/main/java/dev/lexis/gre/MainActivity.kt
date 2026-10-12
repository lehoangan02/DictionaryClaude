package dev.lexis.gre

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File
import android.view.View
import android.speech.tts.TextToSpeech
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
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  private var tts: TextToSpeech? = null

  /** The page asks for a spoken word here when no recording can be played. */
  inner class Voice {
    @JavascriptInterface
    fun speak(word: String) {
      tts?.speak(word, TextToSpeech.QUEUE_FLUSH, null, "word")
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
