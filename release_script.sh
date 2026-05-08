#!/bin/bash
echo "-----------------------------------------------------"
echo "DO NOT RUN UNLESS YOU KNOW THE CODE COMPILES CLEANLY"
echo "THIS CAN TAKE 15 MINUTES TO COMPLETE"
echo "-----------------------------------------------------"
echo "Waiting for Github Actions to complete..."
while true; do
  STATUS=$(gh run list --branch v0.8.2 --json status,conclusion -q '.[0].status')
  CONCLUSION=$(gh run list --branch v0.8.2 --json status,conclusion -q '.[0].conclusion')
  if [ "$STATUS" == "completed" ]; then
    break
  fi
  sleep 10
done

if [ "$CONCLUSION" == "success" ]; then
  echo "CI pipeline completed successfully."
  
  echo "Downloading Windows artifact..."
  gh run download -n RustTracker-Windows --dir ./windows_release
  
  echo "Downloading Linux RPM artifact..."
  gh run download -n RustTracker-Linux-RPM --dir ./linux_rpm
  
  echo "Downloading Linux DEB artifact..."
  gh run download -n RustTracker-Linux-DEB --dir ./linux_deb

  echo "Creating GitHub Release..."
  gh release create v0.8.2 ./windows_release/*.exe ./linux_rpm/*.rpm ./linux_deb/*.deb --title "RustTracker v0.8.2" --notes "Release v0.8.2"
fi
echo "Done!"
