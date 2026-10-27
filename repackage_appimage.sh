#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

UPDATE_TOOLS=false
SKIP_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --update-tools) UPDATE_TOOLS=true ;;
        --skip-build)   SKIP_BUILD=true ;;
    esac
done

LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"

if [ "$UPDATE_TOOLS" = true ] || [ ! -x "./linuxdeploy-x86_64.AppImage" ]; then
    echo "Downloading latest linuxdeploy-x86_64.AppImage..."
    curl -fL -o linuxdeploy-x86_64.AppImage "$LINUXDEPLOY_URL"
    chmod +x linuxdeploy-x86_64.AppImage
fi

if [ "$UPDATE_TOOLS" = true ] || [ ! -x "./appimagetool-x86_64.AppImage" ]; then
    echo "Downloading latest appimagetool-x86_64.AppImage..."
    curl -fL -o appimagetool-x86_64.AppImage "$APPIMAGETOOL_URL"
    chmod +x appimagetool-x86_64.AppImage
fi

VERSION=$(grep -m 1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
TAG="v$VERSION"
OUTPUT_APPIMAGE="RustTracker-SteamDeck-$TAG.AppImage"

if [ "$SKIP_BUILD" = false ]; then
    echo "Building RustTracker $TAG (release)..."
    cargo build --release -j16
fi

echo "Preparing AppDir for RustTracker $TAG..."
rm -rf AppDir

# Populate AppDir and bundle shared library dependencies via linuxdeploy
APPIMAGE_EXTRACT_AND_RUN=1 ./linuxdeploy-x86_64.AppImage \
    --appdir AppDir \
    -e target/release/rusttracker \
    -d rusttracker.desktop \
    -i assets/icons/hicolor/512x512/apps/rusttracker.png

# Install full hicolor icon hierarchy
for sz in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    if [ -f "assets/icons/hicolor/$sz/apps/rusttracker.png" ]; then
        mkdir -p "AppDir/usr/share/icons/hicolor/$sz/apps"
        cp "assets/icons/hicolor/$sz/apps/rusttracker.png" "AppDir/usr/share/icons/hicolor/$sz/apps/rusttracker.png"
    fi
done

echo "Removing conflicting host driver/system libraries from AppDir..."
find AppDir/usr/lib -maxdepth 1 \( \
    -name 'libstdc++*' -o \
    -name 'libgcc_s*' -o \
    -name 'libm.so*' -o \
    -name 'libc.so*' -o \
    -name 'libdl.so*' -o \
    -name 'libpthread*' -o \
    -name 'librt.so*' -o \
    -name 'libresolv*' -o \
    -name 'libnss*' -o \
    -name 'libvulkan*' -o \
    -name 'libdrm*' -o \
    -name 'libva*' -o \
    -name 'libvdpau*' -o \
    -name 'libOpenCL*' -o \
    -name 'libGL*' -o \
    -name 'libEGL*' -o \
    -name 'libgbm*' -o \
    -name 'libwayland*' -o \
    -name 'libX11*' -o \
    -name 'libXext*' -o \
    -name 'libXrender*' -o \
    -name 'libXrandr*' -o \
    -name 'libXfixes*' -o \
    -name 'libXcursor*' -o \
    -name 'libXi*' -o \
    -name 'libXinerama*' -o \
    -name 'libXss*' -o \
    -name 'libXxf86vm*' -o \
    -name 'libXau*' -o \
    -name 'libXdmcp*' -o \
    -name 'libxcb*' -o \
    -name 'libxkbcommon*' -o \
    -name 'libasound*' -o \
    -name 'libpulse*' -o \
    -name 'libpipewire*' -o \
    -name 'libjack*' -o \
    -name 'libudev*' -o \
    -name 'libsystemd*' -o \
    -name 'libdbus*' \
\) -delete || true

echo "Injecting custom AppRun wrapper..."
rm -f AppDir/AppRun
cp AppRun.template AppDir/AppRun
chmod +x AppDir/AppRun

echo "Injecting SoundFont asset and AppStream metadata into AppDir..."
mkdir -p AppDir/usr/share/rusttracker/assets
cp assets/soundfont.sf2 AppDir/usr/share/rusttracker/assets/

# Align desktop and AppStream metadata filenames to rDNS ID (com.soddentrough.rusttracker)
# so appstreamcli and appimagetool validate cleanly with zero warnings.
rm -f AppDir/*.desktop AppDir/usr/share/applications/*.desktop
cp rusttracker.desktop AppDir/usr/share/applications/com.soddentrough.rusttracker.desktop
ln -s usr/share/applications/com.soddentrough.rusttracker.desktop AppDir/com.soddentrough.rusttracker.desktop
mkdir -p AppDir/usr/share/metainfo
sed 's|rusttracker\.desktop|com.soddentrough.rusttracker.desktop|' \
    com.soddentrough.rusttracker.metainfo.xml > AppDir/usr/share/metainfo/com.soddentrough.rusttracker.appdata.xml

# Remove any outdated RustTracker-SteamDeck-v*.AppImage builds in the workspace root
for old_img in RustTracker-SteamDeck-v*.AppImage; do
    if [ -e "$old_img" ] && [ "$old_img" != "$OUTPUT_APPIMAGE" ]; then
        echo "Removing outdated AppImage: $old_img"
        rm -f "$old_img"
    fi
done

echo "Building $OUTPUT_APPIMAGE..."
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 ./appimagetool-x86_64.AppImage AppDir "$OUTPUT_APPIMAGE"
chmod +x "$OUTPUT_APPIMAGE"

echo "Cleaning up intermediate AppDir..."
rm -rf AppDir rusttracker.png

echo "Successfully built $OUTPUT_APPIMAGE!"
