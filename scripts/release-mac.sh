#!/bin/bash
# Signed macOS release for sharing: dist/Growsociety-<version>-macos-arm64.zip.
# Release builds play online against production by default (net.rs PROD_SERVER).
#
# usage: scripts/release-mac.sh [version]        (default 0.11.0)
# Notarization uses APPLE_API_KEY/APPLE_API_KEY_PATH/APPLE_API_ISSUER (App Store Connect key) if set,
# else a notarytool keychain profile named "growsociety":
#   xcrun notarytool store-credentials growsociety --apple-id <id> --team-id XT3J546N4R --password <app-specific>
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-0.11.0}"
identity="${SIGN_IDENTITY:-Developer ID Application: CIPTONINGAJI GURIDNO (XT3J546N4R)}"
zip="dist/Growsociety-$version-macos-arm64.zip"

while pgrep -qf "cargo (build|run|check|test|clippy)"; do
  echo "another cargo build is running; waiting..."
  sleep 10
done
cargo build --release -p game_client

app=dist/Growsociety.app
rm -rf "$app" "$zip"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "${CARGO_TARGET_DIR:-target}/release/game_client" "$app/Contents/MacOS/"
cp -R assets "$app/Contents/Resources/"
cp assets/icon/icon.icns "$app/Contents/Resources/icon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Growsociety</string>
  <key>CFBundleDisplayName</key><string>Growsociety</string>
  <key>CFBundleIdentifier</key><string>app.growsociety.client</string>
  <key>CFBundleExecutable</key><string>game_client</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Voice chat with players near you.</string>
</dict></plist>
PLIST

# Hardened runtime needs the audio-input entitlement for voice chat.
entitlements=$(mktemp)
cat > "$entitlements" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>com.apple.security.device.audio-input</key><true/>
</dict></plist>
XML
codesign --force --deep --options runtime --timestamp --entitlements "$entitlements" -s "$identity" "$app"
rm -f "$entitlements"
codesign --verify --deep --strict --verbose=2 "$app"

ditto -c -k --keepParent "$app" "$zip"
if [ -n "${APPLE_API_KEY:-}" ]; then
  notary_auth=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
elif xcrun notarytool history --keychain-profile growsociety >/dev/null 2>&1; then
  notary_auth=(--keychain-profile growsociety)
fi
if [ -n "${notary_auth:-}" ]; then
  xcrun notarytool submit "$zip" "${notary_auth[@]}" --wait
  xcrun stapler staple "$app"
  rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
  spctl -a -vv "$app"
else
  echo "NOT NOTARIZED: no APPLE_API_* env and no 'growsociety' notarytool profile (see the header of this script)"
fi
ls -lh "$zip"
