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

        private val SUPPORTED_MIME_TYPES = arrayOf(
            "audio/*",
            "video/*",
            "application/ogg",
            "application/x-ogg",
            "audio/midi",
            "audio/x-midi",
            "audio/mid",
            "audio/sp-midi",
            "application/midi",
            "application/x-midi",
            "audio/x-mod",
            "audio/mod",
            "audio/x-xm",
            "audio/xm",
            "audio/x-s3m",
            "audio/s3m",
            "audio/x-it",
            "audio/it",
            "audio/x-stm",
            "audio/x-med",
            "audio/x-mptm",
            "audio/x-669",
            "audio/x-mtm",
            "audio/x-far",
            "audio/x-ult",
            "audio/x-okt",
            "application/x-mod",
            "application/x-xm",
            "application/x-s3m",
            "application/x-it"
        )
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
                openDocumentLauncher.launch(SUPPORTED_MIME_TYPES)
            } catch (e: Exception) {
                Log.e(TAG, "Error launching openDocumentLauncher with media filters", e)
            }
        }
    }

    private fun handleSelectedUri(uri: Uri) {
        Thread {
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

                val safeName = displayName.replace(Regex("[^a-zA-Z0-9._ -]"), "_")
                val finalFile = File(cacheDir, "track_${System.currentTimeMillis()}_$safeName")

                val inputStream = try {
                    contentResolver.openInputStream(uri)
                } catch (e: Exception) {
                    if (decodedPath != null) {
                        try {
                            FileInputStream(File(decodedPath))
                        } catch (e2: Exception) {
                            null
                        }
                    } else {
                        null
                    }
                }

                if (inputStream == null) {
                    Log.e(TAG, "Could not open input stream for URI: $uri")
                    return@Thread
                }

                inputStream.use { input ->
                    FileOutputStream(finalFile).use { output ->
                        input.copyTo(output)
                    }
                }

                if (!finalFile.exists() || finalFile.length() == 0L) {
                    Log.e(TAG, "Cached file is missing or empty: ${finalFile.absolutePath}")
                    return@Thread
                }

                // Clean up old cached files (older than 1 hour)
                try {
                    val oneHourAgo = System.currentTimeMillis() - 3600_000L
                    cacheDir.listFiles()?.forEach { file ->
                        if (file.name.startsWith("track_") && file != finalFile && file.lastModified() < oneHourAgo) {
                            file.delete()
                        }
                    }
                } catch (_: Exception) {}

                Log.i(TAG, "Selected file successfully saved to cache: ${finalFile.absolutePath} (${finalFile.length()} bytes)")
                nativeOnFileSelected(finalFile.absolutePath)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to cache and process selected URI: $uri", e)
            }
        }.start()
    }

    private external fun nativeOnFileSelected(filePath: String)
}
