#!/bin/bash
echo "Waiting for Github Actions to complete..."
while true; do
  STATUS=$(gh run list --branch v0.8.1 --json status,conclusion -q '.[0].status')
  CONCLUSION=$(gh run list --branch v0.8.1 --json status,conclusion -q '.[0].conclusion')
  if [ "$STATUS" == "completed" ]; then
    if [ "$CONCLUSION" == "success" ]; then
      echo "Workflow succeeded!"
      break
    else
      echo "Workflow failed!"
      exit 1
    fi
  fi
  sleep 10
done

echo "Downloading artifacts..."
gh run download -n RustTracker-Windows --dir ./windows_release
gh run download -n RustTracker-Linux-RPM --dir ./linux_rpm
gh run download -n RustTracker-Linux-DEB --dir ./linux_deb

echo "Creating GitHub Release..."
gh release create v0.8.1 ./windows_release/*.exe ./linux_rpm/*.rpm ./linux_deb/*.deb --title "RustTracker v0.8.1" --notes "Release v0.8.1"
echo "Done!"
