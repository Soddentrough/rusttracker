#!/bin/bash
set -e

# Wait for all runs on branch release/v0.8.10 to complete
echo "Waiting for all GH runs to complete..."
while true; do
    RUNS=$(gh run list --branch release/v0.8.10 --json status -q '.[0:3] | map(.status) | join(",")')
    if [[ ! "$RUNS" =~ "in_progress" ]] && [[ ! "$RUNS" =~ "queued" ]]; then
        break
    fi
    echo "Still running: $RUNS"
    sleep 30
done

echo "Runs completed! Downloading artifacts..."
gh run download -n RustTracker-MacOS --dir ./release_artifacts_0.8.10/MacOS || true
gh run download -n RustTracker-Linux-RPM --dir ./release_artifacts_0.8.10/RPM || true
gh run download -n RustTracker-Linux-DEB --dir ./release_artifacts_0.8.10/DEB || true
gh run download -n RustTracker-Windows --dir ./release_artifacts_0.8.10/Windows || true

echo "Uploading to Github Release v0.8.10..."
gh release upload v0.8.10 \
    ./release_artifacts_0.8.10/MacOS/*.dmg \
    ./release_artifacts_0.8.10/RPM/*.rpm \
    ./release_artifacts_0.8.10/DEB/*.deb \
    ./release_artifacts_0.8.10/Windows/*.exe \
    ./RustTracker-SteamDeck-v0.8.10.AppImage \
    --clobber

echo "Done!"
