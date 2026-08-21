#!/usr/bin/env bash
set -euo pipefail

# RustTracker Android Build Script
# Usage: ./scripts/build_android.sh [--release] [--target <abi>]

BUILD_MODE="debug"
CARGO_FLAGS=""
TARGET_ABI="arm64-v8a"
RUST_TARGET="aarch64-linux-android"

# Export Android SDK/NDK paths if not set
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
if [[ -z "${ANDROID_NDK_HOME:-}" && -d "$ANDROID_HOME/ndk" ]]; then
    LATEST_NDK=$(ls -1d "$ANDROID_HOME/ndk/"* 2>/dev/null | sort -V | tail -n 1 || true)
    if [[ -n "$LATEST_NDK" ]]; then
        export ANDROID_NDK_HOME="$LATEST_NDK"
        export NDK_HOME="$LATEST_NDK"
    fi
fi

# Ensure cargo is in PATH
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
        --target)
            TARGET_ABI="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1"
            echo "Usage: $0 [--release] [--target <arm64-v8a|x86_64>]"
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
    echo "Warning: Built library not found at $SO_SOURCE"
fi

echo "==================================================="
echo "  Rust Library Build Completed!"
echo "==================================================="
