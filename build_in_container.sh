#!/bin/bash
set -e
export DEBIAN_FRONTEND=noninteractive

# Install dependencies
apt-get update
apt-get install -y curl build-essential pkg-config libasound2-dev libwayland-dev libx11-dev libxkbcommon-dev libudev-dev libavformat-dev libavfilter-dev libavdevice-dev libswscale-dev libopenmpt-dev clang file wget imagemagick

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

# Build RustTracker natively against Ubuntu 22.04 ffmpeg libraries
echo 'Building RustTracker...'
cargo build --release --target-dir target_appimage

# Resize icon to 512x512 to appease linuxdeploy, and name it to match the desktop file
convert icon.png -resize 512x512! rusttracker.png

# Clean previous AppDir
rm -rf AppDir

# Run linuxdeploy to gather dependencies
APPIMAGE_EXTRACT_AND_RUN=1 ./linuxdeploy-x86_64.AppImage --appdir AppDir -e target_appimage/release/rusttracker -d rusttracker.desktop -i rusttracker.png

echo 'Forcibly removing conflicting host libraries from AppDir...'
find AppDir -name 'libstdc++*' -delete || true
find AppDir -name 'libgcc_s*' -delete || true
find AppDir -name 'libxkbcommon*' -delete || true
find AppDir -name 'libwayland*' -delete || true
find AppDir -name 'libasound*' -delete || true
find AppDir -name 'libvulkan*' -delete || true
find AppDir -name 'libxcb*' -delete || true
find AppDir -name 'libX11*' -delete || true
find AppDir -name 'libXext*' -delete || true
find AppDir -name 'libXrender*' -delete || true
find AppDir -name 'libXrandr*' -delete || true
find AppDir -name 'libXfixes*' -delete || true
find AppDir -name 'libXcursor*' -delete || true
find AppDir -name 'libXi*' -delete || true
find AppDir -name 'libm.so*' -delete || true

echo 'Injecting custom AppRun wrapper to force Wayland...'
# linuxdeploy normally creates AppRun as a symlink to usr/bin/rusttracker. We replace it.
rm -f AppDir/AppRun
cp AppRun.template AppDir/AppRun
chmod +x AppDir/AppRun

echo 'Building final AppImage...'
VERSION=$(grep -m 1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
TAG="v$VERSION"
APPIMAGE_EXTRACT_AND_RUN=1 ./appimagetool-x86_64.AppImage AppDir RustTracker-SteamDeck-$TAG.AppImage

echo 'Done!'
