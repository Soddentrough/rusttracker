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
            "audio/x-s3m",
            "audio/s3m",
            "audio/x-xm",
            "audio/xm",
            "audio/x-it",
            "audio/it",
            "application/x-mod",
            "application/x-s3m",
            "application/x-xm",
            "application/x-it",
            "application/octet-stream",
            "*/*"
        )
    }

    private val currentLoadId = java.util.concurrent.atomic.AtomicLong(0L)
    @Volatile private var isPickerOpen = false
    @Volatile private var lastPickerLaunchMs = 0L

    private val openDocumentLauncher = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        isPickerOpen = false
        uri?.let { handleSelectedUri(it) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        instance = this
        
        ensureSoundfontExtracted()

        // Keep screen awake during visualizations and playback
        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        // Configure edge-to-edge full screen
        WindowCompat.setDecorFitsSystemWindows(window, false)
        val insetsController = WindowCompat.getInsetsController(window, window.decorView)
        insetsController.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE

        onBackPressedDispatcher.addCallback(this, object : androidx.activity.OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (!nativeOnBackPressed()) {
                    finish()
                }
            }
        })

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

    override fun onResume() {
        super.onResume()
        isPickerOpen = false
    }

    override fun onDestroy() {
        if (instance == this) {
            instance = null
        }
        nativeOnDestroy()
        super.onDestroy()
        android.os.Process.killProcess(android.os.Process.myPid())
    }

    fun openFilePicker() {
        runOnUiThread {
            val now = System.currentTimeMillis()
            if (isPickerOpen || (now - lastPickerLaunchMs) < 800L) {
                Log.w(TAG, "Ignoring duplicate openFilePicker request (isPickerOpen=$isPickerOpen)")
                return@runOnUiThread
            }
            try {
                isPickerOpen = true
                lastPickerLaunchMs = now
                openDocumentLauncher.launch(SUPPORTED_MIME_TYPES)
            } catch (e: Exception) {
                isPickerOpen = false
                Log.e(TAG, "Error launching openDocumentLauncher with media filters", e)
            }
        }
    }

    fun getDisplayRefreshRate(): Float {
        return try {
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
                display?.refreshRate ?: 60.0f
            } else {
                @Suppress("DEPRECATION")
                windowManager.defaultDisplay.refreshRate
            }
        } catch (_: Exception) {
            60.0f
        }
    }

    private fun handleSelectedUri(uri: Uri) {
        val loadId = currentLoadId.incrementAndGet()
        Thread {
            var tempFile: File? = null
            val isWaitingForProvider = java.util.concurrent.atomic.AtomicBoolean(false)
            try {
                var displayName = "audio_track"
                var expectedSize = -1L
                val decodedPath = uri.path?.let { Uri.decode(it) }
                if (uri.scheme == "content") {
                    try {
                        contentResolver.query(
                            uri,
                            arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE),
                            null,
                            null,
                            null
                        )?.use { cursor ->
                            if (cursor.moveToFirst()) {
                                val nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                                if (nameIdx >= 0 && !cursor.isNull(nameIdx)) {
                                    val name = cursor.getString(nameIdx)
                                    if (!name.isNullOrBlank()) {
                                        displayName = name
                                    }
                                }
                                val sizeIdx = cursor.getColumnIndex(OpenableColumns.SIZE)
                                if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                                    expectedSize = cursor.getLong(sizeIdx)
                                }
                            }
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "Failed to query DISPLAY_NAME/SIZE for URI: $uri", e)
                    }
                } else if (uri.scheme == "file" && decodedPath != null) {
                    val f = File(decodedPath)
                    displayName = f.name
                    if (f.exists()) {
                        expectedSize = f.length()
                    }
                }

                if (currentLoadId.get() != loadId) return@Thread

                // Immediately notify Rust so the previous track stops playing and UI reflects loading state
                nativeOnFileLoadingStarted(displayName)

                val safeName = displayName.replace(Regex("[^a-zA-Z0-9._ -]"), "_")

                // Reuse existing cached file if same name and exact non-zero size already exists in cacheDir
                if (expectedSize > 0L) {
                    val existing = cacheDir.listFiles()?.firstOrNull { f ->
                        f.isFile &&
                            !f.name.endsWith(".part") &&
                            f.name.startsWith("track_") &&
                            f.name.endsWith("_$safeName") &&
                            f.length() == expectedSize
                    }
                    if (existing != null) {
                        existing.setLastModified(System.currentTimeMillis())
                        Log.i(TAG, "Reusing already-cached file: ${existing.absolutePath} ($expectedSize bytes)")
                        nativeOnFileSelected(existing.absolutePath)
                        return@Thread
                    }
                }

                val sizeMbStr = if (expectedSize >= 1024L * 1024L) {
                    String.format(java.util.Locale.US, " (%.1f MB)", expectedSize / (1024.0 * 1024.0))
                } else {
                    ""
                }
                nativeOnFileStatus("Loading $displayName$sizeMbStr...")

                // Cloud providers (e.g. Google Drive) often download the remote file inside openInputStream()
                // before returning the stream. Run a 1s heartbeat so the user sees live elapsed cloud progress.
                val openStartMs = System.currentTimeMillis()
                isWaitingForProvider.set(true)
                Thread {
                    while (isWaitingForProvider.get() && currentLoadId.get() == loadId) {
                        try {
                            Thread.sleep(1000L)
                        } catch (_: InterruptedException) {
                            break
                        }
                        if (isWaitingForProvider.get() && currentLoadId.get() == loadId) {
                            val elapsed = ((System.currentTimeMillis() - openStartMs) / 1000L).coerceAtLeast(1L)
                            nativeOnFileStatus("Cloud download: $displayName$sizeMbStr (${elapsed}s)...")
                        }
                    }
                }.apply { isDaemon = true }.start()

                val inputStream = try {
                    contentResolver.openInputStream(uri)
                } catch (e: Exception) {
                    Log.w(TAG, "openInputStream failed for $uri, trying AssetFileDescriptor fallback", e)
                    null
                } ?: try {
                    contentResolver.openAssetFileDescriptor(uri, "r")?.createInputStream()
                } catch (e: Exception) {
                    Log.w(TAG, "openAssetFileDescriptor failed for $uri, trying TypedAssetFileDescriptor fallback", e)
                    null
                } ?: try {
                    contentResolver.openTypedAssetFileDescriptor(uri, "*/*", null)?.createInputStream()
                } catch (e: Exception) {
                    Log.w(TAG, "openTypedAssetFileDescriptor failed for $uri", e)
                    null
                } ?: if (decodedPath != null) {
                    try {
                        FileInputStream(File(decodedPath))
                    } catch (e2: Exception) {
                        null
                    }
                } else {
                    null
                }
                isWaitingForProvider.set(false)

                if (currentLoadId.get() != loadId) {
                    inputStream?.close()
                    return@Thread
                }

                if (inputStream == null) {
                    Log.e(TAG, "Could not open input stream for URI: $uri")
                    nativeOnFileStatus("Failed to open cloud/storage file")
                    return@Thread
                }

                val partFile = File(cacheDir, "track_${System.currentTimeMillis()}_${loadId}_$safeName.part")
                tempFile = partFile
                var bytesCopied = 0L
                var lastProgressMs = System.currentTimeMillis()

                inputStream.use { input ->
                    FileOutputStream(partFile).use { output ->
                        val buffer = ByteArray(65536)
                        var read: Int
                        while (input.read(buffer).also { read = it } != -1) {
                            if (currentLoadId.get() != loadId) {
                                Log.i(TAG, "Aborting superseded download (loadId=$loadId)")
                                partFile.delete()
                                return@Thread
                            }
                            output.write(buffer, 0, read)
                            bytesCopied += read
                            val now = System.currentTimeMillis()
                            if (now - lastProgressMs >= 250L) {
                                lastProgressMs = now
                                val copiedMb = bytesCopied / (1024.0 * 1024.0)
                                if (expectedSize > 0L) {
                                    val pct = ((bytesCopied * 100L) / expectedSize).coerceIn(0L, 100L)
                                    val totalMb = expectedSize / (1024.0 * 1024.0)
                                    nativeOnFileStatus(
                                        String.format(
                                            java.util.Locale.US,
                                            "Caching %s... %d%% (%.1f / %.1f MB)",
                                            displayName,
                                            pct,
                                            copiedMb,
                                            totalMb
                                        )
                                    )
                                } else {
                                    nativeOnFileStatus(
                                        String.format(
                                            java.util.Locale.US,
                                            "Caching %s... %.1f MB",
                                            displayName,
                                            copiedMb
                                        )
                                    )
                                }
                            }
                        }
                        output.flush()
                    }
                }

                if (currentLoadId.get() != loadId) {
                    partFile.delete()
                    return@Thread
                }

                if (!partFile.exists() || partFile.length() == 0L) {
                    Log.e(TAG, "Downloaded part file is missing or empty: ${partFile.absolutePath}")
                    partFile.delete()
                    nativeOnFileStatus("Downloaded file is empty")
                    return@Thread
                }

                val finalFile = File(cacheDir, "track_${System.currentTimeMillis()}_$safeName")
                if (!partFile.renameTo(finalFile)) {
                    Log.e(TAG, "Failed to rename ${partFile.absolutePath} to ${finalFile.absolutePath}")
                    partFile.delete()
                    nativeOnFileStatus("Failed to save cached file")
                    return@Thread
                }

                // Clean up previous completed cached media files (never delete active .part files)
                try {
                    val oldTracks = cacheDir.listFiles()
                        ?.filter { it.name.startsWith("track_") && !it.name.endsWith(".part") && it != finalFile }
                        ?.sortedByDescending { it.lastModified() }
                        ?: emptyList()
                    oldTracks.drop(1).forEach { file ->
                        file.delete()
                    }
                } catch (_: Exception) {}

                Log.i(TAG, "Selected file successfully saved to cache: ${finalFile.absolutePath} (${finalFile.length()} bytes)")
                nativeOnFileSelected(finalFile.absolutePath)
            } catch (e: Exception) {
                isWaitingForProvider.set(false)
                tempFile?.delete()
                Log.e(TAG, "Failed to cache and process selected URI: $uri", e)
                if (currentLoadId.get() == loadId) {
                    nativeOnFileStatus("Failed to load file")
                }
            }
        }.start()
    }

    private external fun nativeOnFileLoadingStarted(displayName: String)
    private external fun nativeOnFileSelected(filePath: String)
    private external fun nativeOnFileStatus(statusMsg: String)
    private external fun nativeOnBackPressed(): Boolean
    private external fun nativeOnDestroy()
}
