#!/usr/bin/env bash
set -euo pipefail

# RustTracker Android Build Script
# Usage: ./scripts/build_android.sh [--release|--debug] [--target <abi>] [--apk] [--install] [--run] [--device <serial>]

if [[ -n "${RUST_ALREADY_BUILT:-}" ]]; then
    echo "RustTracker: native library already built, skipping preBuild invocation."
    exit 0
fi

BUILD_MODE="debug"
CARGO_FLAGS=""
TARGET_ABI="arm64-v8a"
RUST_TARGET="aarch64-linux-android"
BUILD_APK=false
DO_INSTALL=false
DO_RUN=false
DEVICE_SERIAL=""

# Export Android SDK/NDK paths if not set
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
if [[ -z "${ANDROID_NDK_HOME:-}" && -d "$ANDROID_HOME/ndk" ]]; then
    LATEST_NDK=$(ls -1d "$ANDROID_HOME/ndk/"* 2>/dev/null | sort -V | tail -n 1 || true)
    if [[ -n "$LATEST_NDK" ]]; then
        export ANDROID_NDK_HOME="$LATEST_NDK"
        export NDK_HOME="$LATEST_NDK"
    fi
fi

# Ensure ~/.local/bin, platform-tools, and cargo are in PATH
export PATH="$HOME/.local/bin:$ANDROID_HOME/platform-tools:$HOME/.cargo/bin:$PATH"
if [[ -f "$HOME/.cargo/env" ]]; then
    source "$HOME/.cargo/env"
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            BUILD_MODE="release"
            CARGO_FLAGS="--release"
            shift
            ;;
        --debug)
            BUILD_MODE="debug"
            CARGO_FLAGS=""
            shift
            ;;
        --target)
            TARGET_ABI="$2"
            shift 2
            ;;
        --apk)
            BUILD_APK=true
            shift
            ;;
        --install)
            BUILD_APK=true
            DO_INSTALL=true
            shift
            ;;
        --run)
            BUILD_APK=true
            DO_INSTALL=true
            DO_RUN=true
            shift
            ;;
        --device)
            DEVICE_SERIAL="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [options]"
            echo ""
            echo "Options:"
            echo "  --release           Build in release mode (default: debug)"
            echo "  --debug             Build in debug mode"
            echo "  --target <abi>      Target ABI: arm64-v8a (default), x86_64, armeabi-v7a"
            echo "  --apk               Package APK using Gradle after building native library"
            echo "  --install           Build APK and install onto connected Android device"
            echo "  --run               Build APK, install, and launch RustTracker on device"
            echo "  --device <serial>   Specify target device serial (optional)"
            echo "  -h, --help          Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown argument: $1"
            echo "Usage: $0 [--release|--debug] [--target <abi>] [--apk] [--install] [--run] [--device <serial>]"
            exit 1
            ;;
    esac
done

case "$TARGET_ABI" in
    "arm64-v8a")
        RUST_TARGET="aarch64-linux-android"
        ;;
    "x86_64")
        RUST_TARGET="x86_64-linux-android"
        ;;
    "armeabi-v7a")
        RUST_TARGET="armv7-linux-androideabi"
        ;;
    *)
        echo "Unsupported ABI: $TARGET_ABI"
        exit 1
        ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "==================================================="
echo "  Building RustTracker for Android"
echo "  Mode:    $BUILD_MODE"
echo "  ABI:     $TARGET_ABI ($RUST_TARGET)"
echo "  SDK:     $ANDROID_HOME"
echo "  NDK:     ${ANDROID_NDK_HOME:-Not found yet}"
echo "==================================================="

# 1. Build Rust cdylib with cargo-ndk
if command -v cargo-ndk &> /dev/null; then
    echo "Building with cargo-ndk..."
    cargo ndk --target "$RUST_TARGET" --platform 29 build $CARGO_FLAGS --lib
else
    echo "Building with standard cargo..."
    cargo build --target "$RUST_TARGET" $CARGO_FLAGS --lib
fi

# 2. Deploy jniLibs
JNILIBS_DIR="android/app/src/main/jniLibs/$TARGET_ABI"
mkdir -p "$JNILIBS_DIR"

SO_SOURCE="target/$RUST_TARGET/$BUILD_MODE/librusttracker.so"
if [[ -f "$SO_SOURCE" ]]; then
    cp -v "$SO_SOURCE" "$JNILIBS_DIR/librusttracker.so"
    echo "Successfully copied librusttracker.so to $JNILIBS_DIR"
else
    echo "Error: Built library not found at $SO_SOURCE" >&2
    exit 1
fi

echo "==================================================="
echo "  Rust Library Build Completed!"
echo "==================================================="

# 3. Optional: Package APK with Gradle
if [[ "$BUILD_APK" = true ]]; then
    echo "==================================================="
    echo "  Building Android APK with Gradle"
    echo "==================================================="
    
    GRADLE_TASK="assembleDebug"
    if [[ "$BUILD_MODE" == "release" ]]; then
        GRADLE_TASK="assembleRelease"
    fi
    
    (cd android && RUST_ALREADY_BUILT=1 ./gradlew "$GRADLE_TASK")
    
    APK_PATH="android/app/build/outputs/apk/$BUILD_MODE/app-$BUILD_MODE.apk"
    if [[ ! -f "$APK_PATH" ]]; then
        echo "Error: Expected APK not found at $APK_PATH" >&2
        exit 1
    fi
    echo "APK created successfully: $APK_PATH"
fi

# 4. Optional: Install APK via ADB
if [[ "$DO_INSTALL" = true ]]; then
    echo "==================================================="
    echo "  Installing to Android Device"
    echo "==================================================="
    
    if ! command -v adb &> /dev/null; then
        echo "Error: adb command not found. Ensure Android platform-tools or ~/.local/bin is installed." >&2
        exit 1
    fi
    
    ADB_CMD=(adb)
    if [[ -n "$DEVICE_SERIAL" ]]; then
        ADB_CMD+=(-s "$DEVICE_SERIAL")
    fi
    
    # Verify device connectivity
    DEVICES_OUTPUT=$("${ADB_CMD[@]}" devices | grep -v "List of devices" | grep "device$" || true)
    if [[ -z "$DEVICES_OUTPUT" ]]; then
        echo "Error: No authorized Android devices connected." >&2
        "${ADB_CMD[@]}" devices -l
        exit 1
    fi
    
    echo "Installing $APK_PATH..."
    "${ADB_CMD[@]}" install -r -d "$APK_PATH"
    echo "Installation successful!"
fi

# 5. Optional: Launch application
if [[ "$DO_RUN" = true ]]; then
    echo "==================================================="
    echo "  Launching RustTracker on Device"
    echo "==================================================="
    
    ADB_CMD=(adb)
    if [[ -n "$DEVICE_SERIAL" ]]; then
        ADB_CMD+=(-s "$DEVICE_SERIAL")
    fi
    
    "${ADB_CMD[@]}" shell am start -n com.rusttracker.app/com.rusttracker.app.MainActivity
    echo "RustTracker launched on device!"
fi
