package tech.carbonkit.relix

import android.app.NativeActivity
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.window.OnBackInvokedDispatcher

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
