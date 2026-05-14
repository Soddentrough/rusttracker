#!/bin/bash
echo "-----------------------------------------------------"
echo "DO NOT RUN UNLESS YOU KNOW THE CODE COMPILES CLEANLY"
echo "THIS CAN TAKE 15 MINUTES TO COMPLETE"
echo "-----------------------------------------------------"
VERSION=$(grep -m 1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
TAG="v$VERSION"

echo "Waiting for Github Actions to complete for branch $TAG..."
while true; do
  STATUS=$(gh run list --branch "$TAG" --json status,conclusion -q '.[0].status')
  CONCLUSION=$(gh run list --branch "$TAG" --json status,conclusion -q '.[0].conclusion')
  if [ "$STATUS" == "completed" ]; then
    break
  fi
  sleep 10
done

if [ "$CONCLUSION" == "success" ]; then
  echo "CI pipeline completed successfully."
   
  echo "Downloading Linux RPM artifact..."
  gh run download -n RustTracker-Linux-RPM --dir ./linux_rpm
  
  echo "Creating GitHub Release..."
  gh release create "$TAG" ./windows_release/*.exe ./linux_rpm/*.rpm ./linux_deb/*.deb ./RustTracker-SteamDeck-$TAG.AppImage --title "RustTracker $TAG" --notes "Release $TAG"
fi
echo "Done!"
