package com.rusttracker.app

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Log
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.google.androidgamesdk.GameActivity
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream

class MainActivity : GameActivity() {

    companion object {
        private const val TAG = "RustTrackerMainActivity"

        init {
            System.loadLibrary("rusttracker")
        }

        @JvmStatic
        var instance: MainActivity? = null
            private set

        @JvmStatic
        fun requestFilePicker() {
            instance?.let { act ->
                act.runOnUiThread {
                    act.openFilePicker()
                }
            } ?: Log.e(TAG, "MainActivity instance is null when requesting file picker")
        }
    }

    private val openDocumentLauncher = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        uri?.let { handleSelectedUri(it) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        instance = this
        
        ensureSoundfontExtracted()

        // Configure edge-to-edge full screen
        WindowCompat.setDecorFitsSystemWindows(window, false)
        val insetsController = WindowCompat.getInsetsController(window, window.decorView)
        insetsController.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE

        intent?.data?.let { uri ->
            handleSelectedUri(uri)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        intent.data?.let { uri ->
            handleSelectedUri(uri)
        }
    }

    private fun ensureSoundfontExtracted() {
        val target = File(filesDir, "soundfont.sf2")
        if (!target.exists() || target.length() == 0L) {
            try {
                assets.open("soundfont.sf2").use { input ->
                    FileOutputStream(target).use { output ->
                        input.copyTo(output)
                    }
                }
                Log.i(TAG, "SoundFont extracted to: ${target.absolutePath}")
            } catch (e: Exception) {
                Log.e(TAG, "Failed to extract soundfont.sf2 from assets", e)
            }
        }
    }

    override fun onDestroy() {
        if (instance == this) {
            instance = null
        }
        super.onDestroy()
    }

    fun openFilePicker() {
        runOnUiThread {
            try {
                openDocumentLauncher.launch(arrayOf("*/*"))
            } catch (e: Exception) {
                Log.e(TAG, "Error launching openDocumentLauncher", e)
            }
        }
    }

    private fun handleSelectedUri(uri: Uri) {
        try {
            var displayName = "audio_track"
            val decodedPath = uri.path?.let { Uri.decode(it) }
            if (uri.scheme == "content") {
                contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        if (nameIdx >= 0) {
                            val name = cursor.getString(nameIdx)
                            if (!name.isNullOrBlank()) {
                                displayName = name
                            }
                        }
                    }
                }
            } else if (uri.scheme == "file" && decodedPath != null) {
                displayName = File(decodedPath).name
            }

            val targetFile = File(cacheDir, displayName)
            if (uri.scheme == "file" && decodedPath != null) {
                val srcFile = File(decodedPath)
                if (srcFile.exists() && srcFile.canonicalPath == targetFile.canonicalPath) {
                    Log.i(TAG, "File already in cache: ${targetFile.absolutePath}")
                    nativeOnFileSelected(targetFile.absolutePath)
                    return
                }
            }

            val inputStream = if (uri.scheme == "file" && decodedPath != null) {
                FileInputStream(File(decodedPath))
            } else {
                contentResolver.openInputStream(uri)
            }

            inputStream?.use { input ->
                FileOutputStream(targetFile).use { output ->
                    input.copyTo(output)
                }
            }

            Log.i(TAG, "Selected file successfully saved to cache: ${targetFile.absolutePath}")
            nativeOnFileSelected(targetFile.absolutePath)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to cache and process selected URI: $uri", e)
        }
    }

    private external fun nativeOnFileSelected(filePath: String)
}
