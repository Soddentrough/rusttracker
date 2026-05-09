#!/bin/bash
set -e

echo "Repackaging AppImage in Ubuntu 22.04 container..."

wget -c -nv "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage"
wget -c -nv "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x linuxdeploy-x86_64.AppImage appimagetool-x86_64.AppImage

chmod +x build_in_container.sh
podman run --rm -v "$PWD":/workspace:z -w /workspace ubuntu:22.04 bash build_in_container.sh
