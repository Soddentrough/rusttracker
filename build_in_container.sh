#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

# When executed inside a fresh Debian/Ubuntu environment (e.g. CI), ensure required packages and Rust exist
if command -v apt-get >/dev/null 2>&1 && ! command -v cargo >/dev/null 2>&1; then
    apt-get update
    apt-get install -y curl build-essential pkg-config libasound2-dev libwayland-dev libx11-dev libxkbcommon-dev libudev-dev libavformat-dev libavfilter-dev libavdevice-dev libswscale-dev libopenmpt-dev clang file wget
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi

exec ./repackage_appimage.sh "$@"
