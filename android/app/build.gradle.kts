plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.rusttracker.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.rusttracker.app"
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.9.15"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters.addAll(listOf("arm64-v8a", "x86_64"))
        }
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

