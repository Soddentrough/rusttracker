plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

@DisableCachingByDefault(because = "Syncing two small text files locally is faster than cache lookup")
abstract class SyncLicenseAssetsTask : DefaultTask() {
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val licenseFiles: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @get:Inject
    abstract val fileSystemOperations: FileSystemOperations

    @TaskAction
    fun sync() {
        fileSystemOperations.sync {
            from(licenseFiles)
            into(outputDir)
        }
    }
}

val syncLicenseAssets = tasks.register("syncLicenseAssets")

android {
    namespace = "com.rusttracker.app"
    compileSdk = 35
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "com.rusttracker.app"
        minSdk = 29
        targetSdk = 35
        versionCode = 39
        versionName = "0.9.39"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters.addAll(listOf("arm64-v8a"))
        }
    }

    lint {
        checkReleaseBuilds = false
        abortOnError = false
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
            signingConfig = signingConfigs.getByName("debug")
        }
        debug {
            isDebuggable = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
            assets.srcDirs("src/main/assets")
        }
    }
}

androidComponents {
    onVariants { variant ->
        val capName = variant.name.replaceFirstChar { it.uppercase() }
        val variantSyncTask = tasks.register<SyncLicenseAssetsTask>("sync${capName}LicenseAssets") {
            val rootDir = rootProject.projectDir.parentFile
            licenseFiles.from(File(rootDir, "LICENSE"), File(rootDir, "THIRD_PARTY_NOTICES.txt"))
        }
        syncLicenseAssets.configure {
            dependsOn(variantSyncTask)
        }
        variant.sources.assets?.addGeneratedSourceDirectory(
            variantSyncTask,
            SyncLicenseAssetsTask::outputDir
        )
    }
}

kotlin {
    jvmToolchain(17)
}


dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.games:games-activity:3.0.5")
    implementation("androidx.media3:media3-session:1.5.1")
    implementation("androidx.media3:media3-common:1.5.1")
}

tasks.register<Exec>("buildRust") {
    val rootDir = rootProject.projectDir.parentFile
    commandLine("bash", "-c", "cd '${rootDir.absolutePath}' && ./scripts/build_android.sh --release --target arm64-v8a")
}

tasks.named("preBuild") {
    dependsOn("buildRust")
}

