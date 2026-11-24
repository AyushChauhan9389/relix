package tech.carbonkit.relix

import android.app.NativeActivity
import android.content.Intent
import android.content.pm.ActivityInfo
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.provider.Settings
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.window.OnBackInvokedDispatcher
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import java.io.File

// Relix is all Rust (android_main in crates/relix-app); this gives its surface key focus, as
// Freya's Android example does, and hands it the Back gesture.
class MainActivity : NativeActivity() {
    companion object {
        // The same library NativeActivity runs, loaded here too so `nativeBack` resolves.
        init {
            System.loadLibrary("relix_app")
        }
    }

    private val handler = Handler(Looper.getMainLooper())
    private val endProcess = Runnable { Process.killProcess(Process.myPid()) }

    /** Back for the app: closes the player or goes back a page; on Home it sends Relix to the background. */
    private external fun nativeBack()

    /**
     * Hands a downloaded update (update.rs, in the app's files folder) to Android's installer. Android
     * asks once to let Relix install apps: that switch opens first, and Install is pressed again after.
     */
    @Suppress("unused") // Called from Rust (lib.rs android_install).
    fun installApk(path: String) {
        runOnUiThread {
            if (Build.VERSION.SDK_INT >= 26 && !packageManager.canRequestPackageInstalls()) {
                try {
                    startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")))
                    return@runOnUiThread
                } catch (_: Exception) {
                    // Some TVs have no such screen; the installer then asks itself.
                }
            }
            val uri = FileProvider.getUriForFile(this, "$packageName.files", File(path))
            val install = Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            startActivity(install)
        }
    }

    private var playing = false

    /**
     * While a video plays: landscape (either way up), with the status and navigation bars hidden
     * (a swipe from the edge shows them for a moment) and the picture allowed beside the camera
     * cutout. After, the screen follows the device again and the bars come back.
     */
    @Suppress("unused") // Called from Rust (lib.rs android_playing).
    fun setPlaying(on: Boolean) {
        runOnUiThread {
            playing = on
            requestedOrientation = if (on) ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE else ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
            window.attributes = window.attributes.apply {
                layoutInDisplayCutoutMode = if (on) WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES else WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_DEFAULT
            }
            applyBars()
        }
    }

    private fun applyBars() {
        val bars = WindowCompat.getInsetsController(window, window.decorView)
        if (playing) {
            bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            bars.hide(WindowInsetsCompat.Type.systemBars())
        } else {
            bars.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    // Coming back from another app (or a dialog) shows the bars again; a playing video hides them.
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && playing) applyBars()
    }

    private fun findNativeSurfaceView(view: View): View? {
        if (view is SurfaceView) return view
        if (view is ViewGroup) {
            for (i in 0 until view.childCount) {
                findNativeSurfaceView(view.getChildAt(i))?.let { return it }
            }
        }
        return null
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.decorView.post {
            findNativeSurfaceView(window.decorView)?.apply {
                isFocusable = true
                isFocusableInTouchMode = true
                requestFocus()
            }
        }
        // Android 13+ delivers Back (the gesture too) here instead of as a key, and its default
        // closes the activity.
        if (Build.VERSION.SDK_INT >= 33) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT) { nativeBack() }
        }
    }

    // Android freezes apps a while after they leave the screen, and Relix's window didn't come back
    // from that (stuck on the splash screen). So a few seconds in the background end it before the
    // freeze, and reopening starts fresh; a quick switch away and back keeps everything.
    override fun onStop() {
        super.onStop()
        handler.postDelayed(endProcess, 5_000)
    }

    override fun onStart() {
        super.onStart()
        handler.removeCallbacks(endProcess)
    }

    override fun onDestroy() {
        super.onDestroy()
        // The UI library runs one event loop per process, so a new activity in this process could
        // never start (stuck on the splash screen). End the process with the activity; the next
        // launch starts fresh. Progress is saved as it happens.
        Process.killProcess(Process.myPid())
    }
}
