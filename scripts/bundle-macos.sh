#!/usr/bin/env bash
# Build "Elyra Workspace.app" (release) with the app icon and bundle metadata.
#   scripts/bundle-macos.sh            -> target/release/bundle/Elyra Workspace.app
# Signs ad hoc unless CODESIGN_IDENTITY is set (e.g. "Developer ID Application: ...");
# with an identity it signs with the hardened runtime and a secure timestamp, as
# notarization requires. scripts/release-macos.sh builds the notarized DMG.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="Elyra Workspace"
BUNDLE_ID="com.gets.elyra-workspace"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$ROOT/target/release/bundle/$NAME.app"

# Apple Silicon only.
if [ "$(uname -m)" != "arm64" ]; then
  echo "Elyra Workspace supports Apple Silicon only; build on an arm64 Mac." >&2
  exit 1
fi

cargo build --release --manifest-path "$ROOT/Cargo.toml" -p elyra-app

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/elyra" "$APP/Contents/MacOS/elyra"
cp "$ROOT/assets/icon/icon.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>elyra</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSArchitecturePriority</key><array><string>arm64</string></array>
  <key>LSRequiresNativeExecution</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSHumanReadableCopyright</key><string>© $(date +%Y) Knut W. Horne</string>
</dict>
</plist>
PLIST

if [ -n "${CODESIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
else
  codesign --force --sign - "$APP"
fi
echo "Built $APP"
