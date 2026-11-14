#!/bin/bash
# Fast local macOS bundle for playtesting: dist/Growsociety.app, online against production.
#
# Same layout as the CI release (.github/workflows/ci.yml), but built with the dev profile: dependencies are
# already compiled at opt-level 3 in target/debug, so only our own crates rebuild (minutes, not a full LTO
# release). Dev builds start offline, so Info.plist's LSEnvironment sets GAME_SERVER to production; the
# client then uses the pinned prod certificate (net.rs). Not for distribution: use a v* tag for that.
#
# usage: scripts/bundle-mac-local.sh [--install]   (--install also copies it to /Applications;
#        GAME_SERVER=host:port overrides the target server)
set -euo pipefail
cd "$(dirname "$0")/.."

server="${GAME_SERVER:-76.13.196.13:7777}"

# One build at a time on the shared target/ (owner rule); wait for other sessions' cargo to finish.
while pgrep -qf "cargo (build|run|check|test|clippy)"; do
  echo "another cargo build is running; waiting..."
  sleep 10
done

cargo build -p game_client   # default features (voice on); never `--features dev` (dylibs aren't bundled)

app=dist/Growsociety.app/Contents
rm -rf dist/Growsociety.app
mkdir -p "$app/MacOS" "$app/Resources"
cp "${CARGO_TARGET_DIR:-target}/debug/game_client" "$app/MacOS/"
cp -R assets "$app/Resources/"
cp assets/icon/icon.icns "$app/Resources/icon.icns"
cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Growsociety</string>
  <key>CFBundleDisplayName</key><string>Growsociety (local)</string>
  <key>CFBundleIdentifier</key><string>app.growsociety.client.local</string>
  <key>CFBundleExecutable</key><string>game_client</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.0-local</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Voice chat with players near you.</string>
  <key>LSEnvironment</key><dict><key>GAME_SERVER</key><string>${server}</string></dict>
</dict></plist>
PLIST

# LaunchServices caches Info.plist (incl. LSEnvironment); re-register so a rebuilt app picks up changes.
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
"$lsregister" -f dist/Growsociety.app
echo "built dist/Growsociety.app (server ${server}); open it with: open dist/Growsociety.app"

if [ "${1:-}" = "--install" ]; then
  rm -rf /Applications/Growsociety.app
  ditto dist/Growsociety.app /Applications/Growsociety.app
  "$lsregister" -f /Applications/Growsociety.app
  echo "installed /Applications/Growsociety.app"
fi
