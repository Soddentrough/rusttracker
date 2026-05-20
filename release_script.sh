#!/bin/bash
set -e
echo "-----------------------------------------------------"
echo "DO NOT RUN UNLESS YOU KNOW THE CODE COMPILES CLEANLY"
echo "THIS CAN TAKE 15 MINUTES TO COMPLETE"
echo "-----------------------------------------------------"
VERSION=$(grep -m 1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
TAG="v$VERSION"

SHA=$(git rev-parse "$TAG")

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
  
  echo "Creating GitHub Release..."
  gh release create "$TAG" ./windows_release/*.exe ./linux_rpm/*.rpm ./linux_deb/*.deb ./macos_release/*.dmg ./RustTracker-SteamDeck-$TAG.AppImage --title "RustTracker $TAG" --notes "Release $TAG"
fi
echo "Done!"
