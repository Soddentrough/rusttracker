#!/bin/bash
set -e
echo "-----------------------------------------------------"
echo "DO NOT RUN UNLESS YOU KNOW THE CODE COMPILES CLEANLY"
echo "THIS CAN TAKE 15 MINUTES TO COMPLETE"
echo "-----------------------------------------------------"
VERSION=$(grep -m 1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
TAG="v$VERSION"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-16}"

SHA=$(git rev-parse "$TAG^{commit}")

echo "Waiting for Github Actions to register runs for commit $SHA ($TAG)..."
while true; do
  TOTAL_RUNS=$(gh run list --commit "$SHA" --json status -q 'length')
  if [ "$TOTAL_RUNS" -gt 0 ]; then
    break
  fi
  echo "Waiting for runs to start..."
  sleep 5
done

echo "Waiting for Github Actions to complete for commit $SHA..."
while true; do
  ACTIVE_RUNS=$(gh run list --commit "$SHA" --json status -q 'map(select(.status != "completed")) | length')
  if [ "$ACTIVE_RUNS" -eq 0 ]; then
    break
  fi
  echo "Still running: $ACTIVE_RUNS workflows active..."
  sleep 10
done

TOTAL_RUNS=$(gh run list --commit "$SHA" --json conclusion -q 'length')
SUCCESS_RUNS=$(gh run list --commit "$SHA" --json conclusion -q 'map(select(.conclusion == "success")) | length')

if [ "$SUCCESS_RUNS" -eq "$TOTAL_RUNS" ] && [ "$TOTAL_RUNS" -gt 0 ]; then
  echo "CI pipeline completed successfully."
   
  echo "Cleaning old artifact directories..."
  rm -rf ./linux_rpm ./linux_deb ./windows_release ./macos_release
  
  echo "Downloading artifacts..."
  gh run download -n RustTracker-Linux-RPM --dir ./linux_rpm || true
  gh run download -n RustTracker-Linux-DEB --dir ./linux_deb || true
  gh run download -n RustTracker-Windows --dir ./windows_release || true
  gh run download -n RustTracker-MacOS --dir ./macos_release || true
  
  if gh release view "$TAG" >/dev/null 2>&1; then
    echo "GitHub Release $TAG exists. Uploading/clobbering desktop artifacts..."
    gh release upload "$TAG" ./windows_release/*.exe ./linux_rpm/*.rpm ./linux_deb/*.deb ./macos_release/*.dmg --clobber || true
  else
    echo "Creating GitHub Release $TAG..."
    EXTRA_FILES=()
    if [ -f "./RustTracker-SteamDeck-$TAG.AppImage" ]; then
      EXTRA_FILES+=("./RustTracker-SteamDeck-$TAG.AppImage")
    fi
    gh release create "$TAG" ./windows_release/*.exe ./linux_rpm/*.rpm ./linux_deb/*.deb ./macos_release/*.dmg "${EXTRA_FILES[@]}" --title "RustTracker $TAG" --notes "Release $TAG"
  fi

  # Check for attached Android devices and automatically deploy
  echo "-----------------------------------------------------"
  echo "Checking for attached Android phone/device..."
  echo "-----------------------------------------------------"
  export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
  export PATH="$HOME/.local/bin:$ANDROID_HOME/platform-tools:$HOME/.cargo/bin:$PATH"

  if command -v adb >/dev/null 2>&1; then
    ATTACHED_DEVICES=$(adb devices 2>/dev/null | awk '$2 == "device" { print $1 }' || true)
    if [ -n "$ATTACHED_DEVICES" ]; then
      echo "Attached Android device(s) detected:"
      echo "$ATTACHED_DEVICES"
      echo "Building and deploying latest release APK to attached device(s)..."
      ./scripts/build_android.sh --release --apk --auto-deploy
      echo "Android auto-deployment completed successfully!"
    else
      echo "No attached Android devices detected. Skipping Android deployment."
    fi
  else
    echo "adb not found. Skipping Android deployment."
  fi
fi
echo "Release workflow completed!"
